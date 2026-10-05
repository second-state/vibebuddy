//! Everything the app sees comes from here: link, device, today's stats, integrations, the operation in progress.
//! The device's mode, firmware build id and current voice are all read from its diagnostic lines.

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::activity::TodaySummary;
use crate::config::Config;
use crate::serial_transport::DeviceMessage;

/// What the device looks like right now, pieced together from diagnostic lines; a device reboot reports it all again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceState {
    pub connected: bool,
    pub port: Option<String>,
    /// Connected through the UART bridge (rather than Espressif's native USB port).
    pub bridge: bool,
    /// duty / pomodoro / leisure
    pub mode: Option<String>,
    pub firmware_build: Option<String>,
    /// Id of the announcement voice the device is using; `builtin` means the built-in voice.
    pub voice: Option<String>,
    /// Speaker volume (the codec's 20 to 100), stored on the device; the app is just a remote.
    pub volume: Option<u8>,
}

impl DeviceState {
    /// Takes in one device message; returns whether the state changed.
    pub fn apply(&mut self, message: &DeviceMessage) -> bool {
        let before = self.clone();
        match message {
            DeviceMessage::Connected { port, bridge } => {
                self.connected = true;
                self.port = Some(port.clone());
                self.bridge = *bridge;
                // A newly connected port starts with unknown identity: our firmware re-reports its build as soon as it gets hello;
                // one that never does runs other firmware (a factory unit), and the app offers to flash it. A build id left by
                // the previous box mustn't pass for this one.
                self.firmware_build = None;
            }
            DeviceMessage::Disconnected => {
                self.connected = false;
                self.port = None;
            }
            DeviceMessage::Line(line) => {
                if let Some(mode) = line.strip_prefix("MODE ") {
                    self.mode = Some(mode.trim().to_ascii_lowercase());
                // Anywhere in the line: a line the box wrote before the port was opened can lose its newline and
                // arrive glued in front ("LEISURE SKIT DISPLAY READY BUILD …", seen 2026-10-01), and missing the
                // build hides the firmware update.
                } else if let Some((_, build)) = line.split_once("DISPLAY READY BUILD ") {
                    self.firmware_build = Some(build.trim().to_owned());
                } else if let Some(voice) = line.strip_prefix("VOICES ") {
                    let voice = voice.trim();
                    if voice != "NO PARTITION" {
                        self.voice = Some(voice.to_owned());
                    }
                } else if let Some(volume) = line.strip_prefix("VOLUME ") {
                    // `VOLUME ERROR` has no number to parse, so the volume stays as it was.
                    if let Ok(level) = volume.trim().parse::<u8>() {
                        self.volume = Some(level);
                    }
                }
            }
            DeviceMessage::Event(_) => {}
        }
        *self != before
    }
}

/// When each of the two agents last sent a hook.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HooksSeen {
    pub codex: Option<DateTime<Local>>,
    pub claude: Option<DateTime<Local>>,
}

/// The device operation in progress: writing a voice pack or flashing firmware, only one at a time.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Operation {
    pub kind: OperationKind,
    pub state: OperationState,
    /// 0 to 1.
    pub progress: f32,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    VoicePack,
    Firmware,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Running,
    Done,
    Failed,
    /// Firmware written, but the box didn't start it; it needs unplugging and plugging back in.
    Replug,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonInfo {
    pub build: String,
    pub app_version: Option<String>,
}

/// What `GET /v1/status` returns and the status stream pushes.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub daemon: DaemonInfo,
    pub device: DeviceState,
    pub today: TodaySummary,
    pub hooks: HooksSeen,
    pub operation: Option<Operation>,
    pub config: Config,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> DeviceMessage {
        DeviceMessage::Line(text.to_owned())
    }

    #[test]
    fn a_build_glued_behind_a_stray_line_is_still_read() {
        let mut state = DeviceState::default();
        state.apply(&line("LEISURE SKIT DISPLAY READY BUILD v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
        assert_eq!(state.firmware_build.as_deref(), Some("v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
    }

    #[test]
    fn device_state_is_read_off_the_diagnostic_lines() {
        let mut state = DeviceState::default();
        assert!(state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true }));
        assert!(state.apply(&line("DISPLAY READY BUILD 21a8360-dirty 2026-09-16 10:23")));
        assert!(state.apply(&line("VOICES builtin")));
        assert!(state.apply(&line("MODE POMODORO")));
        assert!(!state.apply(&line("MODE POMODORO")), "no change is not a change");
        assert!(state.apply(&line("VOLUME 65")));
        assert!(!state.apply(&line("VOLUME ERROR")), "an error does not change the volume");
        assert_eq!(
            state,
            DeviceState {
                connected: true,
                port: Some("/dev/cu.x".to_owned()),
                bridge: true,
                mode: Some("pomodoro".to_owned()),
                firmware_build: Some("21a8360-dirty 2026-09-16 10:23".to_owned()),
                voice: Some("builtin".to_owned()),
                volume: Some(65),
            }
        );
    }

    #[test]
    fn disconnecting_keeps_the_last_known_firmware_and_voice() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true });
        state.apply(&line("DISPLAY READY BUILD abc 2026-09-16 10:23"));
        state.apply(&line("VOICES wanwanxiaohe"));
        assert!(state.apply(&DeviceMessage::Disconnected));
        assert!(!state.connected);
        assert_eq!(state.port, None);
        assert_eq!(state.firmware_build.as_deref(), Some("abc 2026-09-16 10:23"));
        assert_eq!(state.voice.as_deref(), Some("wanwanxiaohe"));
    }

    #[test]
    fn a_new_connection_forgets_the_previous_firmware_build() {
        // A different factory unit got plugged in; the previous box's build id mustn't make the app think it's ours.
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true });
        state.apply(&line("DISPLAY READY BUILD abc 2026-09-16 10:23"));
        state.apply(&DeviceMessage::Disconnected);
        assert!(state.apply(&DeviceMessage::Connected { port: "/dev/cu.y".to_owned(), bridge: false }));
        assert_eq!(state.firmware_build, None);
    }

    #[test]
    fn a_device_without_the_voices_partition_reports_no_voice() {
        let mut state = DeviceState::default();
        assert!(!state.apply(&line("VOICES NO PARTITION")));
        assert_eq!(state.voice, None);
    }
}
