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
    pub config: Config,
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
    fn durations_switch_to_hours() {
        assert_eq!(duration(59), "0 min");
        assert_eq!(duration(3600 + 120), "1 h 2 min");
    }
}
