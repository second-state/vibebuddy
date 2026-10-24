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

/// The daemon accepts the pack and writes it in the background; progress arrives in the status stream.
/// `lender` (the Character lending a custom one its voice, persona and lines) and `address` (the form of address
/// its lines carry) aren't in the pack; the daemon keeps them for the buddy's spoken replies.
pub async fn write_voice_pack(pack: Vec<u8>, lender: Option<&str>, address: Option<&str>) -> Result<(), String> {
    // Both are ids from fixed lists (`ada`, `laoban`): nothing to escape.
    let query: Vec<String> = [("lender", lender), ("address", address)]
        .into_iter()
        .filter_map(|(name, value)| Some(format!("{name}={}", value?)))
        .collect();
    let query = if query.is_empty() { String::new() } else { format!("?{}", query.join("&")) };
    let request = reqwest::Client::new()
        .post(format!("{BASE}/v1/device/voice-pack{query}"))
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
    post("/v1/daemon/restart", None).await
}

/// Whether the daemon runs the experimental resident session (`VIBEBUDDY_RESIDENT_SESSION=1`).
pub async fn resident_enabled() -> Result<bool, String> {
    resident_info().await.map(|info| info.enabled).map_err(|error| match error {
        HistoryError::DaemonDown => "the daemon isn't running".to_owned(),
        HistoryError::Unavailable(why) => why,
    })
}

/// `GET /v1/resident`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ResidentInfo {
    pub enabled: bool,
    /// Goes up when a voice command's turn ends or the thread changes, so the Conversation tab reads the history
    /// again only then. Kept in memory: it starts again at 0 when the daemon restarts. None from an older daemon.
    #[serde(default)]
    pub revision: Option<u64>,
    /// What voice commands are missing while the session is on but not ready; None when ready, and from a daemon
    /// too old to say.
    #[serde(default)]
    pub problem: Option<Problem>,
    /// Which Agent the user wants answering; None from a daemon older than the choice.
    #[serde(default)]
    pub brain_setting: Option<BrainChoice>,
    /// Whether the daemon holds an Anthropic API key; the key itself never comes back.
    #[serde(default)]
    pub anthropic_key_set: Option<bool>,
}

/// Why the resident session can't take voice commands now: voice needs the user's Codex, or the user's Claude Code
/// with their own Anthropic API key.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    CodexMissing,
    CodexLoggedOut,
    ClaudeMissing,
    AnthropicKeyMissing,
    /// A reason this app doesn't know yet.
    #[serde(other)]
    Other,
}

/// Which Agent answers voice commands: Codex when it is ready, else Claude Code when an Anthropic API key is set
/// (`Auto`); or always the one picked.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrainChoice {
    Auto,
    Codex,
    Claude,
    /// A setting this app doesn't know yet.
    #[serde(other)]
    Other,
}

impl BrainChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto | Self::Other => "auto",
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

/// Which Agent answers voice commands. Its own endpoint, not part of the config.
pub async fn set_resident_brain(choice: BrainChoice) -> Result<ResidentInfo, String> {
    let request = reqwest::Client::new().put(format!("{BASE}/v1/resident/brain")).json(&serde_json::json!({"brain": choice.as_str()}));
    resident_setting(request).await
}

/// Stores the user's Anthropic API key in the daemon (None removes it). The daemon never sends it back.
pub async fn set_anthropic_key(key: Option<String>) -> Result<ResidentInfo, String> {
    let client = reqwest::Client::new();
    let url = format!("{BASE}/v1/resident/anthropic-key");
    let request = match key {
        Some(key) => client.put(url).json(&serde_json::json!({"api_key": key})),
        None => client.delete(url),
    };
    resident_setting(request).await
}

/// A provider of spoken replies the user can give their own key for (ADR-0020).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyProvider {
    ElevenLabs,
    Volc,
}

impl KeyProvider {
    pub const ALL: [Self; 2] = [Self::ElevenLabs, Self::Volc];

    /// Its name in the daemon's API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ElevenLabs => "elevenlabs",
            Self::Volc => "volc",
        }
    }
}

/// Sets the user's own key for a provider of spoken replies (None clears it). The daemon never sends it back; the
/// status says it is set.
pub async fn set_voice_key(provider: KeyProvider, key: Option<String>) -> Result<(), String> {
    let request = reqwest::Client::new().put(format!("{BASE}/v1/voice-keys")).json(&serde_json::json!({provider.as_str(): key}));
    let response = request.timeout(Duration::from_secs(10)).send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    Err(body["message"].as_str().map(str::to_owned).unwrap_or_else(|| format!("the daemon answered {status}")))
}

async fn resident_setting(request: reqwest::RequestBuilder) -> Result<ResidentInfo, String> {
    let response = request.timeout(Duration::from_secs(10)).send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(body["message"].as_str().map(str::to_owned).unwrap_or_else(|| format!("the daemon answered {status}")));
    }
    serde_json::from_value(body).map_err(|error| error.to_string())
}

/// Asked every 2 seconds while the Conversation tab shows, so it answers quickly or not at all.
pub async fn resident_info() -> Result<ResidentInfo, HistoryError> {
    let response = reqwest::Client::new()
        .get(format!("{BASE}/v1/resident"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|error| if error.is_connect() { HistoryError::DaemonDown } else { HistoryError::Unavailable(error.to_string()) })?;
    let body = response.bytes().await.map_err(|error| HistoryError::Unavailable(error.to_string()))?;
    serde_json::from_slice(&body).map_err(|error| HistoryError::Unavailable(error.to_string()))
}

/// One voice command and the buddy's spoken replies to it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Exchange {
    pub said: String,
    pub replies: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum HistoryError {
    DaemonDown,
    /// The resident session couldn't be read (Codex missing or not logged in, say), with the daemon's reason.
    Unavailable(String),
}

/// The last `limit` voice commands and their spoken replies, oldest first. Slow the first time: the daemon may have
/// to start Codex's app-server to read the thread.
pub async fn resident_history(limit: usize) -> Result<Vec<Exchange>, HistoryError> {
    #[derive(Deserialize)]
    struct History {
        exchanges: Vec<Exchange>,
    }
    let response = reqwest::Client::new()
        .get(format!("{BASE}/v1/resident/history?limit={limit}"))
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| if error.is_connect() { HistoryError::DaemonDown } else { HistoryError::Unavailable(error.to_string()) })?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(HistoryError::Unavailable(
            response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| body["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("the daemon answered {status}")),
        ));
    }
    let history: History = response.json().await.map_err(|error| HistoryError::Unavailable(error.to_string()))?;
    Ok(history.exchanges)
}

/// Starts signing in to the VibeBuddy account: the daemon answers with the page to open in the browser, which comes
/// back to the daemon, and the status stream then shows the account signed in.
pub async fn sign_in() -> Result<String, String> {
    #[derive(Deserialize)]
    struct SignIn {
        url: String,
    }
    let response = reqwest::Client::new()
        .post(format!("{BASE}/v1/account/sign-in"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(body["message"].as_str().map(str::to_owned).unwrap_or_else(|| format!("the daemon answered {status}")));
    }
    serde_json::from_value::<SignIn>(body).map(|sign_in| sign_in.url).map_err(|error| error.to_string())
}

/// Ends this computer's session only.
pub async fn sign_out() -> Result<(), String> {
    post("/v1/account/sign-out", None).await
}

/// Deletes the account and signs out everywhere; on failure the user stays signed in.
pub async fn delete_account() -> Result<(), String> {
    accepted(reqwest::Client::new().delete(format!("{BASE}/v1/account"))).await
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

    #[test]
    fn the_resident_info_reads_its_revision_and_problem_and_an_older_daemons_without_them() {
        let parse = |json: &str| serde_json::from_str::<ResidentInfo>(json).unwrap();
        let older = parse(r#"{"enabled":true,"thread_id":null}"#);
        let nothing = ResidentInfo { enabled: true, revision: None, problem: None, brain_setting: None, anthropic_key_set: None };
        assert_eq!(older, nothing);
        let ready = parse(r#"{"enabled":true,"thread_id":"t1","revision":7,"ready":true,"problem":null}"#);
        assert_eq!(ready, ResidentInfo { revision: Some(7), ..nothing.clone() });
        let missing = parse(r#"{"enabled":true,"thread_id":null,"revision":0,"ready":false,"problem":"codex_missing"}"#);
        assert_eq!(missing.problem, Some(Problem::CodexMissing));
        let logged_out = parse(r#"{"enabled":true,"thread_id":null,"revision":0,"ready":false,"problem":"codex_logged_out"}"#);
        assert_eq!(logged_out.problem, Some(Problem::CodexLoggedOut));
        let later = parse(r#"{"enabled":true,"thread_id":null,"revision":0,"ready":false,"problem":"something_new"}"#);
        assert_eq!(later.problem, Some(Problem::Other));
    }

    #[test]
    fn the_resident_info_says_which_agent_answers_and_whether_a_key_is_stored() {
        let parse = |json: &str| serde_json::from_str::<ResidentInfo>(json).unwrap();
        let claude = parse(r#"{"enabled":true,"thread_id":"s1","revision":3,"ready":true,"problem":null,"brain":"claude","brain_setting":"auto","anthropic_key_set":true}"#);
        assert_eq!((claude.brain_setting, claude.anthropic_key_set), (Some(BrainChoice::Auto), Some(true)));
        let no_key = parse(r#"{"enabled":true,"problem":"anthropic_key_missing","brain_setting":"claude","anthropic_key_set":false}"#);
        assert_eq!((no_key.problem, no_key.brain_setting), (Some(Problem::AnthropicKeyMissing), Some(BrainChoice::Claude)));
        let no_claude = parse(r#"{"enabled":true,"problem":"claude_missing","brain_setting":"something_new"}"#);
        assert_eq!((no_claude.problem, no_claude.brain_setting), (Some(Problem::ClaudeMissing), Some(BrainChoice::Other)));
        assert_eq!(BrainChoice::Other.as_str(), "auto");
    }
}
