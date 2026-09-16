//! App 看到的一切都来自这里：链路、设备、当日战绩、接入、正在进行的操作。
//! 设备的模式、固件构建标识、当前音色都从它的诊断行里读出来。

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::activity::TodaySummary;
use crate::config::Config;
use crate::serial_transport::DeviceMessage;

/// 设备此刻的样子，按诊断行逐步拼出来；设备重启会重新报一遍。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceState {
    pub connected: bool,
    pub port: Option<String>,
    /// 接在 UART 桥上（而非乐鑫原生 USB 口）。
    pub bridge: bool,
    /// duty / pomodoro / leisure
    pub mode: Option<String>,
    pub firmware_build: Option<String>,
    /// 设备正在用的播报音色 id；`builtin` 表示内置音色。
    pub voice: Option<String>,
}

impl DeviceState {
    /// 吸收一条设备消息；返回状态是否变了。
    pub fn apply(&mut self, message: &DeviceMessage) -> bool {
        let before = self.clone();
        match message {
            DeviceMessage::Connected { port, bridge } => {
                self.connected = true;
                self.port = Some(port.clone());
                self.bridge = *bridge;
            }
            DeviceMessage::Disconnected => {
                self.connected = false;
                self.port = None;
            }
            DeviceMessage::Line(line) => {
                if let Some(mode) = line.strip_prefix("MODE ") {
                    self.mode = Some(mode.trim().to_ascii_lowercase());
                } else if let Some(build) = line.strip_prefix("DISPLAY READY BUILD ") {
                    self.firmware_build = Some(build.trim().to_owned());
                } else if let Some(voice) = line.strip_prefix("VOICES ") {
                    let voice = voice.trim();
                    if voice != "NO PARTITION" {
                        self.voice = Some(voice.to_owned());
                    }
                }
            }
            DeviceMessage::Event(_) => {}
        }
        *self != before
    }
}

/// 两个 Agent 各自最近一次送来 Hook 的时刻。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HooksSeen {
    pub codex: Option<DateTime<Local>>,
    pub claude: Option<DateTime<Local>>,
}

/// 正在进行的设备操作：写语音包或烧固件，同一时刻只有一个。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Operation {
    pub kind: OperationKind,
    pub state: OperationState,
    /// 0 到 1。
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
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonInfo {
    pub build: String,
    pub app_version: Option<String>,
}

/// `GET /v1/status` 与状态流推的都是它。
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
    fn device_state_is_read_off_the_diagnostic_lines() {
        let mut state = DeviceState::default();
        assert!(state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true }));
        assert!(state.apply(&line("DISPLAY READY BUILD 21a8360-dirty 2026-09-16 10:23")));
        assert!(state.apply(&line("VOICES builtin")));
        assert!(state.apply(&line("MODE POMODORO")));
        assert!(!state.apply(&line("MODE POMODORO")), "没变就不算变");
        assert_eq!(
            state,
            DeviceState {
                connected: true,
                port: Some("/dev/cu.x".to_owned()),
                bridge: true,
                mode: Some("pomodoro".to_owned()),
                firmware_build: Some("21a8360-dirty 2026-09-16 10:23".to_owned()),
                voice: Some("builtin".to_owned()),
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
    fn a_device_without_the_voices_partition_reports_no_voice() {
        let mut state = DeviceState::default();
        assert!(!state.apply(&line("VOICES NO PARTITION")));
        assert_eq!(state.voice, None);
    }
}
