//! Talks to `vibebuddyd` over local HTTP only, as the Mac app does: the status arrives as an SSE stream, and
//! every change goes through the daemon's API, never its files.

use std::time::Duration;

use futures::{SinkExt, Stream, StreamExt};
use serde::Deserialize;

use crate::status::{Config, Status};

const BASE: &str = "http://127.0.0.1:7331";
const RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub enum Update {
    Status(Box<Status>),
    /// The daemon isn't answering; the stream keeps retrying on its own.
    Down,
}

/// Snapshots for as long as the app runs, reconnecting whenever the daemon goes away.
pub fn status_updates() -> impl Stream<Item = Update> {
    iced::stream::channel(16, async |mut output| {
        let client = reqwest::Client::new();
        loop {
            if let Ok(response) = client.get(format!("{BASE}/v1/status/stream")).send().await
                && response.status().is_success()
            {
                let mut body = response.bytes_stream();
                let mut buffer = Vec::new();
                while let Some(Ok(chunk)) = body.next().await {
                    buffer.extend_from_slice(&chunk);
                    for status in drain_events(&mut buffer) {
                        let _ = output.send(Update::Status(Box::new(status))).await;
                    }
                }
            }
            let _ = output.send(Update::Down).await;
            tokio::time::sleep(RETRY).await;
        }
    })
}

/// Takes every complete `data:` line out of `buffer`, leaving a partial line for the next chunk.
fn drain_events(buffer: &mut Vec<u8>) -> Vec<Status> {
    let mut statuses = Vec::new();
    while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=end).collect();
        let line = String::from_utf8_lossy(&line);
        if let Some(data) = line.trim_end().strip_prefix("data:")
            && let Ok(status) = serde_json::from_str(data.trim_start())
        {
            statuses.push(status);
        }
    }
    statuses
}

pub async fn put_config(config: Config) -> Result<Config, String> {
    let response = reqwest::Client::new()
        .put(format!("{BASE}/v1/config"))
        .json(&config)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response.json().await.map_err(|error| error.to_string())
}

/// The box reports its new volume back, and that is when the status stream shows it.
pub async fn set_volume(level: u8, preview: bool) -> Result<(), String> {
    post("/v1/device/volume", Some(serde_json::json!({ "level": level, "preview": preview }))).await
}

pub async fn identify() -> Result<(), String> {
    post("/v1/device/identify", None).await
}

pub async fn restart_daemon() -> Result<(), String> {
    post("/v1/daemon/restart", None).await
}

async fn post(path: &str, body: Option<serde_json::Value>) -> Result<(), String> {
    let mut request = reqwest::Client::new().post(format!("{BASE}{path}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if response.status().is_success() {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct Rejection {
        message: String,
    }
    let status = response.status();
    Err(response
        .json::<Rejection>()
        .await
        .map(|rejection| rejection.message)
        .unwrap_or_else(|_| format!("the daemon answered {status}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks_are_reassembled() {
        let mut buffer = b"event: status\ndata: {\"today\":{\"done\":2}}\n\nevent: status\ndata: {\"today\":".to_vec();
        let statuses = drain_events(&mut buffer);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].today.done, 2);
        buffer.extend_from_slice(b"{\"done\":3}}\n\n:keep-alive\n");
        let statuses = drain_events(&mut buffer);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].today.done, 3);
        assert!(buffer.is_empty());
    }
}
