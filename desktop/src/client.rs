//! Talks to `vibebuddyd` over local HTTP only, as the Mac app does: the status arrives as an SSE stream, and
//! every change goes through the daemon's API, never its files.

use std::time::Duration;

use futures::{SinkExt, Stream, StreamExt};
use serde::Deserialize;

use crate::status::{Config, Status};

const BASE: &str = "http://127.0.0.1:7331";
const RETRY: Duration = Duration::from_secs(2);
const STATUS_HEADERS_TIMEOUT: Duration = Duration::from_secs(10);
// The daemon sends SSE keepalives every 15 s, even when no status changes. Allow missed intervals, but never keep
// displaying a stale status forever when the daemon stalls with an open socket.
const STATUS_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug)]
pub enum Update {
    Status(Box<Status>),
    /// The daemon isn't answering; the stream keeps retrying on its own.
    Down,
}

/// Snapshots for as long as the app runs, reconnecting whenever the daemon goes away.
pub fn status_updates() -> impl Stream<Item = Update> {
    status_updates_at(BASE.to_owned())
}

/// The same stream from another base URL, for the headless probe that runs against a test daemon.
pub fn status_updates_at(base: String) -> impl Stream<Item = Update> {
    status_updates_with_timeouts(base, STATUS_HEADERS_TIMEOUT, STATUS_IDLE_TIMEOUT)
}

fn status_updates_with_timeouts(base: String, headers_timeout: Duration, idle_timeout: Duration) -> impl Stream<Item = Update> {
    iced::stream::channel(16, async move |mut output| {
        let client = reqwest::Client::new();
        loop {
            let request = client.get(format!("{base}/v1/status/stream")).send();
            if let Ok(Ok(response)) = tokio::time::timeout(headers_timeout, request).await
                && response.status().is_success()
            {
                let mut body = response.bytes_stream();
                let mut buffer = Vec::new();
                while let Ok(Some(Ok(chunk))) = tokio::time::timeout(idle_timeout, body.next()).await {
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

/// The daemon accepts the pack and writes it in the background; progress arrives in the status stream.
pub async fn write_voice_pack(pack: Vec<u8>) -> Result<(), String> {
    let request = reqwest::Client::new()
        .post(format!("{BASE}/v1/device/voice-pack"))
        .header("content-type", "application/octet-stream")
        .body(pack);
    accepted(request).await
}

/// Like the voice pack, flashing runs in the background; the daemon reads the three images from these paths.
/// Flashes the three images in `directory`: the firmware the daemon downloaded from the update manifest.
pub async fn flash_firmware(directory: std::path::PathBuf) -> Result<(), String> {
    let body = serde_json::json!({
        "bootloader": directory.join("bootloader.bin"),
        "partition_table": directory.join("partition-table.bin"),
        "app": directory.join("vibebuddy-fw.bin"),
    });
    post("/v1/device/firmware", Some(body)).await
}

pub async fn check_for_updates() -> Result<(), String> {
    post("/v1/updates/check", None).await
}

/// The daemon remembers the choice and connects to that device on its next look around.
pub async fn choose_box(usb_serial: String) -> Result<(), String> {
    post("/v1/device/choice", Some(serde_json::json!({ "usb_serial": usb_serial }))).await
}

/// Forgets another computer on the box; the daemon refuses this computer, which pairs again whenever it's plugged in.
pub async fn unpair(key: String) -> Result<(), String> {
    post("/v1/device/unpair", Some(serde_json::json!({ "key": key }))).await
}

/// Takes the box from the computer that has it; the daemon's next try over the network or the Relay asks outright.
pub async fn take_over() -> Result<(), String> {
    post("/v1/device/takeover", None).await
}

/// The password goes to the box over the cable and is kept nowhere else.
pub async fn set_wifi(ssid: String, password: String) -> Result<(), String> {
    post("/v1/device/wifi", Some(serde_json::json!({ "ssid": ssid, "password": password }))).await
}

/// The box scans; what it hears arrives in the status as `wifi_seen`.
pub async fn scan_wifi() -> Result<(), String> {
    post("/v1/device/wifi/scan", None).await
}

pub async fn forget_wifi() -> Result<(), String> {
    post("/v1/device/wifi/forget", None).await
}

pub async fn identify() -> Result<(), String> {
    post("/v1/device/identify", None).await
}

/// The box's current screen as PNG. It holds the serial port for a few seconds over the UART bridge, so it is only
/// ever taken on request.
pub async fn screenshot() -> Result<Vec<u8>, String> {
    let response = reqwest::Client::new()
        .post(format!("{BASE}/v1/device/screenshot"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if status.is_success() {
        Ok(body.to_vec())
    } else {
        Err(String::from_utf8_lossy(&body).trim().to_owned())
    }
}

pub async fn restart_daemon() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // The daemon cannot restart itself over HTTP while it is down. Both Advanced and the tray use its supervisor.
        let command = tokio::process::Command::new("systemctl")
            .args(["--user", "restart", "vibebuddyd"])
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(Duration::from_secs(10), command)
            .await
            .map_err(|_| "systemctl restart timed out".to_owned())?
            .map_err(|error| format!("cannot run systemctl: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl restart failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
    #[cfg(not(target_os = "linux"))]
    post("/v1/daemon/restart", None).await
}

async fn post(path: &str, body: Option<serde_json::Value>) -> Result<(), String> {
    let mut request = reqwest::Client::new().post(format!("{BASE}{path}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    accepted(request).await
}

async fn accepted(request: reqwest::RequestBuilder) -> Result<(), String> {
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Hold the socket open after sending the requested traffic, so EOF cannot accidentally satisfy a timeout test.
    async fn stream_server(headers: bool, keepalives: bool) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request_start = [0; 1];
            socket.read_exact(&mut request_start).await.unwrap();
            if headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: {}\n\n")
                    .await
                    .unwrap();
            }
            if keepalives {
                for _ in 0..8 {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    socket.write_all(b": keep-alive\n\n").await.unwrap();
                }
                socket.write_all(b"data: {\"today\":{\"done\":2}}\n\n").await.unwrap();
            }
            std::future::pending::<()>().await;
            drop(socket);
        });
        (base, server)
    }

    #[tokio::test]
    async fn a_connection_that_never_sends_headers_reports_down() {
        let (base, server) = stream_server(false, false).await;
        let mut updates = std::pin::pin!(status_updates_with_timeouts(base, Duration::from_millis(250), STATUS_IDLE_TIMEOUT));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), updates.next()).await.unwrap(),
            Some(Update::Down)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn an_open_stream_that_stops_sending_bytes_reports_down() {
        let (base, server) = stream_server(true, false).await;
        let mut updates = std::pin::pin!(status_updates_with_timeouts(
            base,
            STATUS_HEADERS_TIMEOUT,
            Duration::from_millis(250)
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), updates.next()).await.unwrap(),
            Some(Update::Status(_))
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), updates.next()).await.unwrap(),
            Some(Update::Down)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn keepalives_preserve_a_stream_longer_than_its_idle_timeout() {
        let (base, server) = stream_server(true, true).await;
        let mut updates = std::pin::pin!(status_updates_with_timeouts(
            base,
            STATUS_HEADERS_TIMEOUT,
            Duration::from_millis(250)
        ));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), updates.next()).await.unwrap(),
            Some(Update::Status(_))
        ));
        match tokio::time::timeout(Duration::from_secs(3), updates.next()).await.unwrap() {
            Some(Update::Status(status)) => assert_eq!(status.today.done, 2),
            update => panic!("keepalives should preserve the stream: {update:?}"),
        }
        server.abort();
    }

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
