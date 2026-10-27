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
    /// What the update manifest offers (ADR-0010); absent from older daemons.
    pub updates: Option<Updates>,
    /// Every Activity, newest first, as the box's task cards show them; empty from older daemons.
    pub activities: Vec<Activity>,
    /// The VibeBuddy account this computer is signed in to (ADR-0018); older daemons don't say, which reads as not
    /// available.
    pub account: Account,
    /// Which of the user's own keys for spoken replies are set (ADR-0020); absent from older daemons.
    pub voice_keys: Option<VoiceKeys>,
    /// The Character being made, or the last one made since the daemon started (ADR-0020); absent when there is none,
    /// and from older daemons.
    pub making: Option<crate::making::Making>,
}

/// The daemon says only whether each key is set, and whether its provider refused it; a key never comes back.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceKeys {
    pub elevenlabs: VoiceKey,
    pub volc: VoiceKey,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceKey {
    pub set: bool,
    pub refused: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Account {
    /// This build of the daemon has a service to sign in to.
    pub available: bool,
    pub signed_in: bool,
    /// An account has an email or a GitHub user, never both.
    pub email: Option<String>,
    pub github_id: Option<u64>,
    /// Looked up on GitHub when signing in; the service keeps only the id.
    pub github_login: Option<String>,
    /// Voice used in the current UTC day, when the daemon knows.
    pub today: Option<Usage>,
    /// Why the service refused voice today: `allowance` or `global_cap`.
    pub refused: Option<String>,
    /// The account's plan (ADR-0020), when the daemon knows.
    pub plan: Option<Plan>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Plan {
    /// `free` or `paid`.
    pub name: String,
    /// When the billing period ends and the plan renews, RFC 3339; the counts and this come only on the Paid plan.
    pub period_end: Option<String>,
    pub characters: Option<u64>,
    pub characters_limit: Option<u64>,
}

impl Plan {
    pub fn paid(&self) -> bool {
        self.name == "paid"
    }

    /// "This month: 12,300 of 100,000 characters · renews 24 Nov", on the Paid plan only.
    pub fn month_line(&self) -> Option<String> {
        let (Some(used), Some(limit), Some(end)) = (self.characters, self.characters_limit, self.period_end.as_deref()) else { return None };
        let end = chrono::DateTime::parse_from_rfc3339(end).ok()?.with_timezone(&chrono::Local);
        let renews = if crate::i18n::is_chinese() { end.format("%-m月%-d日") } else { end.format("%-d %b") }.to_string();
        self.paid().then(|| tr("This month: %@ of %@ characters · renews %@", &[&grouped(used), &grouped(limit), &renews]))
    }
}

/// `12300` as "12,300".
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub day: String,
    pub commands: u64,
    pub commands_limit: u64,
}

/// One Activity as the daemon summarises it (daemon/src/activity.rs `ActivitySummary`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Activity {
    /// `codex`, `claude-code`, `github-actions`, or the agent a CLI hook names (`opencode`, `copilot`, `pi`).
    pub agent: String,
    /// The task card's title, with the prefix that tells agents apart (`CC:`, `CX:`…).
    pub title: String,
    pub project: Option<String>,
    /// `working` or `needs_input`.
    pub state: String,
}

/// The daemon's view of updates; which firmware to offer is its call, as on the Mac.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Updates {
    pub enabled: bool,
    pub last_check: Option<String>,
    pub error: Option<String>,
    pub app: Option<AppOffer>,
    pub unsupported_app: bool,
    pub firmware: Option<FirmwareOffer>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppOffer {
    pub version: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct FirmwareOffer {
    pub version: String,
    pub notes: std::collections::BTreeMap<String, String>,
    /// The unpacked images, once the daemon has downloaded and checked them.
    pub directory: Option<std::path::PathBuf>,
    pub newer_than_box: bool,
}

impl FirmwareOffer {
    /// Release notes in the UI's language, English otherwise.
    pub fn notes(&self, chinese: bool) -> Option<&str> {
        chinese.then(|| self.notes.get("zh-Hans")).flatten().or_else(|| self.notes.get("en")).map(String::as_str)
    }
}

impl Updates {
    /// An update is offered only for a box that runs something older, and only once the firmware is on disk.
    pub fn firmware_update_available(&self) -> bool {
        self.firmware.as_ref().is_some_and(|offer| offer.newer_than_box && offer.directory.is_some())
    }

    /// The images to flash, once downloaded.
    pub fn firmware_directory(&self) -> Option<&std::path::Path> {
        self.firmware.as_ref()?.directory.as_deref()
    }
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
    /// Firmware written, but the box didn't start it; it needs unplugging and plugging back in.
    Replug,
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
            (OperationKind::Firmware, OperationState::Done) => tr("Firmware flashed, the box has restarted", &[]),
            (_, OperationState::Replug) => tr("Firmware flashed, but the box didn't start it. Unplug the box and plug it back in.", &[]),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DaemonInfo {
    pub build: String,
    /// The git description alone; empty from a daemon older than the field.
    pub revision: String,
}

impl DaemonInfo {
    /// What the settings window shows: the revision, or the whole build line from an older daemon.
    pub fn short_build(&self) -> &str {
        if self.revision.is_empty() { &self.build } else { &self.revision }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Device {
    pub connected: bool,
    pub port: Option<String>,
    pub bridge: bool,
    pub mode: Option<String>,
    pub firmware_build: Option<String>,
    /// Firmware older than ADR-0010 and older daemons report none.
    pub firmware_version: Option<String>,
    pub voice: Option<String>,
    pub volume: Option<u8>,
    /// Which set of screens the box draws, `robot` or `korekore` (ADR-0021); None from firmware that predates the choice.
    pub screens: Option<String>,
    /// Linked over the local network or the Relay; `port` is then the box's or the Relay's address.
    pub network: bool,
    pub relay: bool,
    /// Another computer has the box (ADR-0011).
    pub busy: Option<Busy>,
    /// Connected but running other firmware, as the daemon judged it (docs/architecture.md, decision 17).
    pub foreign_firmware: bool,
    /// Our firmware built for other hardware (the breadboard devkit); released firmware isn't offered to it.
    pub unsupported_board: Option<String>,
    /// An environment variable narrowing the search to one port or device; older daemons report none.
    pub pin: Option<Pin>,
    /// Devices that could be the box when there are several and none is the remembered one.
    pub candidates: Vec<Candidate>,
    /// The box's key (ADR-0012); none for firmware that can't pair, or from an older daemon.
    pub box_key: Option<String>,
    /// The firmware takes updates over any link (ADR-0016): no cable, no download mode.
    pub transfer: bool,
    /// The last image sent over the link didn't confirm itself, and the box went back (`FIRMWARE ROLLED BACK`).
    pub rolled_back: bool,
    /// This computer's key, to tell it apart in `paired_computers`.
    pub computer_key: Option<String>,
    pub paired_computers: Vec<PairedComputer>,
    /// The Wi-Fi network the box is set to join, and the address it got there.
    pub wifi_network: Option<String>,
    pub wifi_address: Option<String>,
    /// Why the box couldn't join the network, until it does.
    pub wifi_error: Option<String>,
    /// What the box heard on its last scan, strongest first.
    pub wifi_seen: Vec<SeenNetwork>,
    /// Why the Relay won't carry the link (ADR-0020): `signed_out`, `plan` or `unavailable`.
    pub relay_refused: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SeenNetwork {
    pub ssid: String,
    pub rssi: i32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Busy {
    /// By name; empty when the box didn't say.
    pub owner: String,
    /// `usb`: on that computer's cable, can't be taken. `linked`: can be taken over.
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PairedComputer {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Pin {
    pub variable: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Candidate {
    pub port: String,
    /// What the choice is remembered by; without one the device can't be chosen.
    pub usb_serial: Option<String>,
}

impl Device {
    /// The firmware as shown in the interface: version first, then the build ID.
    pub fn firmware_label(&self) -> Option<String> {
        match (&self.firmware_version, &self.firmware_build) {
            (Some(version), Some(build)) => Some(format!("{version} · {build}")),
            (version, build) => version.clone().or_else(|| build.clone()),
        }
    }

    /// What the link section says about the Relay turning this computer away.
    pub fn relay_refusal(&self) -> Option<String> {
        Some(match self.relay_refused.as_deref()? {
            "signed_out" => tr("Sign in to use the Relay.", &[]),
            "plan" => tr("The Relay needs the Paid plan.", &[]),
            _ => tr("The Relay can't check the account right now. VibeBuddy tries again later.", &[]),
        })
    }
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
    pub opencode: Option<String>,
    pub copilot: Option<String>,
    pub pi: Option<String>,
}

/// The daemon owns this; the app only reads it and writes it back whole through `PUT /v1/config`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Config {
    pub voice: Option<String>,
    pub notify_link: bool,
    /// None follows the build: on in releases CI makes, off when built from source.
    pub check_updates: Option<bool>,
    /// Reach the box through the Relay when nothing closer does (ADR-0012); on unless turned off.
    pub relay: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { voice: None, notify_link: true, check_updates: None, relay: true }
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
                // The tray offers to start it again: systemd may have given up after it failed to start.
                device_line: tr("daemon not responding · click to restart", &[]),
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
    fn the_box_screens_parse() {
        let device: Device = serde_json::from_str(r#"{"connected": true, "port": null, "bridge": false, "screens": "korekore"}"#).expect("device");
        assert_eq!(device.screens.as_deref(), Some("korekore"));
    }

    #[test]
    fn a_live_snapshot_parses() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert!(status.device.connected);
        assert_eq!(status.device.volume, Some(40));
        assert_eq!(status.device.screens, None, "firmware that predates the choice says nothing");
        assert_eq!(status.today.busy_seconds, 2217);
        assert!(status.config.notify_link);
    }

    #[test]
    fn the_settings_window_shows_the_revision_alone() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert_eq!(status.daemon.short_build(), "v0.2.2 2026-10-01 13:23", "an older daemon has no revision");
        let newer: DaemonInfo = serde_json::from_str(
            r#"{"build": "0.4.0 v0.4.0-19-gcb7f43c 2026-10-09 17:28", "revision": "v0.4.0-19-gcb7f43c"}"#,
        )
        .expect("daemon");
        assert_eq!(newer.short_build(), "v0.4.0-19-gcb7f43c");
    }

    #[test]
    fn the_firmware_label_leads_with_the_version() {
        let mut status: Status = serde_json::from_str(LIVE).expect("status");
        assert_eq!(status.device.firmware_label().as_deref(), Some("v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
        status.device.firmware_version = Some("0.2.2".to_owned());
        assert_eq!(status.device.firmware_label().as_deref(), Some("0.2.2 · v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
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
    fn a_flash_waiting_for_a_replug_parses_and_says_so() {
        // An unknown state would fail the whole status and cut the app off from the daemon.
        let operation: Operation = serde_json::from_str(
            r#"{"kind": "firmware", "state": "replug", "progress": 1.0, "message": "firmware flashed but not started, replug the box"}"#,
        )
        .expect("operation");
        assert!(!operation.running());
        assert_eq!(operation.summary(), "Firmware flashed, but the box didn't start it. Unplug the box and plug it back in.");
    }

    #[test]
    fn firmware_is_offered_once_downloaded_and_newer_than_the_box() {
        let updates: Updates = serde_json::from_str(
            r#"{"enabled": true, "firmware": {"version": "0.4.0", "notes": {"en": "- Fix", "zh-Hans": "- 修复"},
                "directory": "/state/updates/firmware/0.4.0", "newer_than_box": true}}"#,
        )
        .expect("updates");
        assert!(updates.firmware_update_available());
        assert_eq!(updates.firmware_directory(), Some(std::path::Path::new("/state/updates/firmware/0.4.0")));
        let offer = updates.firmware.clone().unwrap();
        assert_eq!(offer.notes(true), Some("- 修复"));
        assert_eq!(offer.notes(false), Some("- Fix"));
        let mut not_downloaded = updates.clone();
        not_downloaded.firmware.as_mut().unwrap().directory = None;
        assert!(!not_downloaded.firmware_update_available());
        let mut same = updates;
        same.firmware.as_mut().unwrap().newer_than_box = false;
        assert!(!same.firmware_update_available());
        assert!(!Updates::default().firmware_update_available());
    }

    #[test]
    fn the_relays_refusal_is_explained() {
        let device: Device = serde_json::from_str(r#"{"connected":false,"relay_refused":"plan"}"#).expect("device");
        assert_eq!(device.relay_refusal().as_deref(), Some("The Relay needs the Paid plan."));
        assert_eq!(Device::default().relay_refusal(), None);
    }

    #[test]
    fn the_account_reads_as_not_available_from_an_older_daemon() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert_eq!(status.account, Account::default());
        assert!(!status.account.available);
        let account: Account = serde_json::from_str(
            r#"{"available": true, "signed_in": true, "email": null, "github_id": 60097, "github_login": "octocat"}"#,
        )
        .expect("account");
        assert!(account.available && account.signed_in);
        assert_eq!((account.github_id, account.github_login.as_deref(), account.email), (Some(60097), Some("octocat"), None));
        let used: Account = serde_json::from_str(r#"{"signed_in":true,"today":{"day":"2026-10-24","commands":7,"commands_limit":100},"refused":"global_cap"}"#).expect("account");
        assert_eq!(used.today, Some(Usage { day: "2026-10-24".to_owned(), commands: 7, commands_limit: 100 }));
        assert_eq!(used.refused.as_deref(), Some("global_cap"));
        assert_eq!(used.plan, None);
    }

    #[test]
    fn the_plan_decodes_and_only_the_paid_plan_has_a_month() {
        let free: Account = serde_json::from_str(r#"{"signed_in":true,"plan":{"name":"free"}}"#).expect("account");
        let free = free.plan.expect("plan");
        assert!(!free.paid());
        assert_eq!(free.month_line(), None);
        let paid: Plan = serde_json::from_str(r#"{"name":"paid","period_end":"2026-11-24T12:00:00.000Z","characters":12300,"characters_limit":100000}"#).expect("plan");
        assert!(paid.paid());
        let line = paid.month_line().expect("a month");
        assert!(line.starts_with("This month: 12,300 of 100,000 characters · renews ") && line.ends_with(" Nov"), "{line}");
        assert_eq!((grouped(0), grouped(999), grouped(1000), grouped(1234567)), ("0".to_owned(), "999".to_owned(), "1,000".to_owned(), "1,234,567".to_owned()));
    }

    #[test]
    fn own_keys_read_as_absent_from_an_older_daemon() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert_eq!(status.voice_keys, None);
        let keys: VoiceKeys = serde_json::from_str(r#"{"elevenlabs": {"set": true, "refused": true}, "volc": {"set": false, "refused": false}}"#).expect("keys");
        assert_eq!(keys, VoiceKeys { elevenlabs: VoiceKey { set: true, refused: true }, volc: VoiceKey::default() });
    }

    #[test]
    fn the_character_being_made_reads_and_is_absent_from_an_older_daemon() {
        let status: Status = serde_json::from_str(LIVE).expect("status");
        assert_eq!(status.making, None);
        let status: Status = serde_json::from_str(r#"{"making":{"id":"owl-1a2b3c4d","name":"Owl","state":"making","done":3,"total":12,"reason":null,"error":null}}"#).expect("status");
        let making = status.making.expect("making");
        assert!(making.running() && making.id == "owl-1a2b3c4d" && making.progress() == 0.25, "{making:?}");
    }

    #[test]
    fn durations_switch_to_hours() {
        assert_eq!(duration(59), "0 min");
        assert_eq!(duration(3600 + 120), "1 h 2 min");
    }
}
