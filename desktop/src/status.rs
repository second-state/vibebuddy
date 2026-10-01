//! The daemon's status snapshot, read only as far as the app needs it, and the tray menu derived from it.
//! Mirrors `MenuState.swift` so both apps say the same thing.

use serde::{Deserialize, Serialize};

use crate::i18n::tr;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Status {
    pub daemon: DaemonInfo,
    pub device: Device,
    pub today: Today,
    pub hooks: Hooks,
    pub operation: Option<Operation>,
    pub config: Config,
}

/// Writing a voice pack or flashing firmware; the daemon runs one at a time and reports progress here.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Operation {
    pub kind: OperationKind,
    pub state: OperationState,
    /// 0 to 1.
    pub progress: f32,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    VoicePack,
    Firmware,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Running,
    Done,
    Failed,
}

impl Operation {
    pub fn running(&self) -> bool {
        self.state == OperationState::Running
    }

    /// Said in the UI language; the daemon's own (English) message only shows when something failed.
    pub fn summary(&self) -> String {
        let percent = format!("{:.0}%", self.progress * 100.0);
        match (self.kind, self.state) {
            (_, OperationState::Failed) => tr("Failed: %@", &[&self.message]),
            (OperationKind::VoicePack, OperationState::Running) => tr("Writing voice pack… %@", &[&percent]),
            (OperationKind::VoicePack, OperationState::Done) => tr("Voice pack written", &[]),
            (OperationKind::Firmware, OperationState::Running) => tr("Flashing firmware… %@", &[&percent]),
            (OperationKind::Firmware, OperationState::Done) => tr("Firmware flashed, the box is restarting", &[]),
        }
    }
}

/// Builds are reported as "hash date time"; only the hash says which firmware it is.
pub fn firmware_hash(build: Option<&str>) -> Option<&str> {
    build.and_then(|build| build.split(' ').next()).filter(|hash| !hash.is_empty())
}

/// Offered whenever the hashes differ, without judging which is newer, as on the Mac (docs/app.md). Nothing is offered
/// before the box has reported its build.
pub fn firmware_update_available(device: Option<&str>, bundled: Option<&str>) -> bool {
    matches!((firmware_hash(device), firmware_hash(bundled)), (Some(device), Some(bundled)) if device != bundled)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DaemonInfo {
    pub build: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Device {
    pub connected: bool,
    pub port: Option<String>,
    pub bridge: bool,
    pub mode: Option<String>,
    pub firmware_build: Option<String>,
    pub voice: Option<String>,
    pub volume: Option<u8>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Today {
    pub done: u32,
    pub asks: u32,
    pub busy_seconds: u64,
}

/// When each agent's last hook event arrived, as the daemon wrote it (RFC 3339).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hooks {
    pub codex: Option<String>,
    pub claude: Option<String>,
}

/// The daemon owns this; the app only reads it and writes it back whole through `PUT /v1/config`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Config {
    pub voice: Option<String>,
    pub notify_link: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { voice: None, notify_link: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Online,
    Offline,
    DaemonDown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub icon: Icon,
    pub device_line: String,
    pub mode_line: String,
    pub today_line: String,
}

impl MenuState {
    /// `None` means the daemon isn't answering.
    pub fn derive(status: Option<&Status>) -> Self {
        let Some(status) = status else {
            return Self {
                icon: Icon::DaemonDown,
                device_line: tr("daemon isn't running", &[]),
                mode_line: "—".to_owned(),
                today_line: "—".to_owned(),
            };
        };
        let device = &status.device;
        let device_line = if device.connected {
            let build = device
                .firmware_build
                .as_deref()
                .and_then(|build| build.split(' ').next())
                .unwrap_or("?");
            tr("Box online · firmware %@", &[&build])
        } else {
            tr("Box not found", &[])
        };
        let mode = match device.mode.as_deref() {
            Some("duty") => tr("On duty", &[]),
            Some("pomodoro") => tr("Pomodoro", &[]),
            Some("leisure") => tr("Leisure", &[]),
            _ => "—".to_owned(),
        };
        let today = &status.today;
        Self {
            icon: if device.connected { Icon::Online } else { Icon::Offline },
            device_line,
            mode_line: tr("Mode: %@", &[&mode]),
            today_line: tr(
                "Today: done %lld · asks %lld · busy %@",
                &[&today.done, &today.asks, &duration(today.busy_seconds)],
            ),
        }
    }
}

pub fn duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        tr("%lld h %lld min", &[&hours, &minutes])
    } else {
        tr("%lld min", &[&minutes])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = r#"{
        "daemon": {"build": "v0.2.2 2026-10-01 13:23", "app_version": null},
        "device": {"connected": true, "port": "/dev/ttyACM0", "bridge": false, "mode": "duty",
                   "firmware_build": "v0.2.1-38-ga0bffc7 2026-09-30 16:29", "voice": "xiaohe2", "volume": 40},
        "today": {"done": 8, "asks": 3, "busy_seconds": 2217},
        "hooks": {"codex": "2026-10-01T17:06:59+08:00", "claude": null},
        "operation": null,
        "config": {"voice": null, "notify_link": true}
    }"#;

    #[test]
    fn a_live_snapshot_parses() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert!(status.device.connected);
        assert_eq!(status.device.volume, Some(40));
        assert_eq!(status.today.busy_seconds, 2217);
        assert!(status.config.notify_link);
    }

    #[test]
    fn the_menu_says_what_the_mac_app_says() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        let menu = MenuState::derive(Some(&status));
        assert_eq!(menu.icon, Icon::Online);
        // Tests run without a Chinese locale, so the English key is the text.
        assert_eq!(menu.device_line, "Box online · firmware v0.2.1-38-ga0bffc7");
        assert_eq!(menu.mode_line, "Mode: On duty");
        assert_eq!(menu.today_line, "Today: done 8 · asks 3 · busy 36 min");
    }

    #[test]
    fn no_daemon_and_no_box_read_differently() {
        assert_eq!(MenuState::derive(None).icon, Icon::DaemonDown);
        let offline = Status::default();
        assert_eq!(MenuState::derive(Some(&offline)).icon, Icon::Offline);
        assert_eq!(MenuState::derive(Some(&offline)).device_line, "Box not found");
    }

    #[test]
    fn progress_reads_in_percent_and_failures_quote_the_daemon() {
        let mut operation: Operation = serde_json::from_str(
            r#"{"kind": "voice_pack", "state": "running", "progress": 0.426, "message": "chunk 12/40"}"#,
        )
        .expect("operation");
        assert!(operation.running());
        assert_eq!(operation.summary(), "Writing voice pack… 43%");
        operation.state = OperationState::Failed;
        assert_eq!(operation.summary(), "Failed: chunk 12/40");
    }

    #[test]
    fn firmware_is_compared_by_hash_only() {
        let device = Some("v0.2.1-38-ga0bffc7 2026-09-30 16:29");
        assert!(firmware_update_available(device, Some("v0.2.2 2026-09-30 09:13")));
        assert!(!firmware_update_available(Some("v0.2.2 2026-10-01 10:00"), Some("v0.2.2 2026-09-30 09:13")));
        assert!(!firmware_update_available(None, Some("v0.2.2 2026-09-30 09:13")));
        assert!(!firmware_update_available(device, None));
    }

    #[test]
    fn durations_switch_to_hours() {
        assert_eq!(duration(59), "0 min");
        assert_eq!(duration(3600 + 120), "1 h 2 min");
    }
}
