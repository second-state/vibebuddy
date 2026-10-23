//! Everything the app sees comes from here: link, device, today's stats, integrations, the operation in progress.
//! The device's mode, firmware build id and current voice are all read from its diagnostic lines.

use chrono::{DateTime, Local};
use serde::Serialize;

use crate::activity::TodaySummary;
use crate::config::Config;
use crate::serial_transport::{BUILD_MARKER, Candidate, DeviceMessage, Pin};

/// Reported next to the build, at boot and in answer to hello, so it can arrive glued behind a stray line too.
const VERSION_MARKER: &str = "FIRMWARE VERSION ";
/// The board our firmware says it was built for. Released firmware is built for the box alone and says nothing;
/// a build for other hardware (the breadboard devkit, `goouuu-s3-spi`) says which.
const BOARD_PREFIX: &str = "BOARD ";
/// The box's public key; firmware that can pair reports it with its build.
pub const BOX_KEY_PREFIX: &str = "BOX KEY ";
/// What a build for the box itself calls it, when it does say.
const RELEASE_BOARD: &str = "alientek-box";

/// What the device looks like right now, pieced together from diagnostic lines; a device reboot reports it all again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceState {
    pub connected: bool,
    pub port: Option<String>,
    /// Connected through the UART bridge (rather than Espressif's native USB port).
    pub bridge: bool,
    /// Linked over the local network or the Relay rather than the cable; `port` is then the box's address.
    pub network: bool,
    /// Linked through the Relay; `port` is then the Relay's address.
    pub relay: bool,
    /// The box is linked to another computer, so this one can only take it over (ADR-0011).
    pub busy: Option<Busy>,
    /// duty / pomodoro / leisure
    pub mode: Option<String>,
    pub firmware_build: Option<String>,
    /// The firmware version, such as `0.2.2`, which orders releases; firmware older than ADR-0010 reports none.
    pub firmware_version: Option<String>,
    /// The firmware takes firmware images over any link (`TRANSFER 1`, ADR-0016), so updating needs no cable.
    pub transfer: bool,
    /// The last image sent over the link didn't confirm itself, and the box went back (`FIRMWARE ROLLED BACK`).
    pub rolled_back: bool,
    /// Id of the announcement voice the device is using; `builtin` means the built-in voice.
    pub voice: Option<String>,
    /// Speaker volume (the codec's 20 to 100), stored on the device; the app is just a remote.
    pub volume: Option<u8>,
    /// The connected port's USB serial number, which tells this device from another ESP32-S3.
    pub usb_serial: Option<String>,
    /// The box's public key, its id on the Relay (ADR-0012); None for firmware that can't pair.
    pub box_key: Option<String>,
    /// The box has confirmed it is paired with this computer.
    pub paired: bool,
    /// The box said `VOICE DIRECT`: over the Relay it calls the voice service itself, with a voice token (ADR-0018).
    pub voice_direct: bool,
    /// This computer's own key, so the app can tell it apart in `paired_computers`. Set once at start.
    pub computer_key: Option<String>,
    /// Every computer the box is paired with, as it last listed them.
    pub paired_computers: Vec<PairedComputer>,
    /// The Wi-Fi network the box is set to join, by name; the password stays on the box.
    pub wifi_network: Option<String>,
    /// The address the box got on that network.
    pub wifi_address: Option<String>,
    /// Why the box couldn't join it, from its last `WIFI FAILED <why>` (see [`wifi_failure`]); None once it joins,
    /// the network changes or the user joins again.
    pub wifi_error: Option<String>,
    /// The Relay this computer uses, `wss://host`; None without a key. Set by the daemon, not the box.
    pub relay_server: Option<String>,
    /// The Relay the box uses, as it last said (`RELAY URL`); it follows the computer on the cable.
    pub box_relay: Option<String>,
    /// The networks the box heard on its last scan, strongest first, one entry per name.
    pub wifi_seen: Vec<SeenNetwork>,
    /// Connected, but running other firmware (a factory unit, or Muse on a box that runs it): it's there, not
    /// offline, and the daemon writes it nothing until it resets and reports our build.
    pub foreign_firmware: bool,
    /// Our firmware built for hardware that released firmware doesn't run on. Released firmware is never offered
    /// to it: on 2026-10-08 the update offer flashed the breadboard devkit with box firmware, and its screen,
    /// speaker and buttons all failed to start.
    pub unsupported_board: Option<String>,
    /// Set when an environment variable narrows the search to one port or device; the app says so, or a box
    /// elsewhere would just look unfound.
    pub pin: Option<Pin>,
    /// Devices that could be the box when there are several and none is the remembered one; the user picks.
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Busy {
    /// The computer that has the box, by name; empty when the box didn't say.
    pub owner: String,
    /// `usb`: it is on that computer's cable, and can't be taken. `linked`: it can be taken over.
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SeenNetwork {
    pub ssid: String,
    /// Signal strength in dBm.
    pub rssi: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PairedComputer {
    pub key: String,
    pub name: String,
}

impl DeviceState {
    /// Takes in one device message; returns whether the state changed.
    pub fn apply(&mut self, message: &DeviceMessage) -> bool {
        let before = self.clone();
        match message {
            DeviceMessage::Connected { port, bridge, usb_serial } => {
                self.connected = true;
                self.port = Some(port.clone());
                self.bridge = *bridge;
                // A newly connected port starts with unknown identity: our firmware re-reports its build, mode, voice and volume
                // as soon as it gets hello; one that never does runs other firmware (a factory unit, or another ESP32-S3 product
                // entirely), and the app offers to flash it. Nothing left by the previous device may pass for this one: on
                // 2026-10-06 a leftover voice made another product look like the box, and it nearly got flashed.
                self.usb_serial = usb_serial.clone();
                self.network = false;
                self.relay = false;
                self.busy = None;
                self.unsupported_board = None;
                self.candidates.clear();
                self.foreign_firmware = false;
                self.firmware_build = None;
                self.firmware_version = None;
                self.transfer = false;
                self.rolled_back = false;
                self.mode = None;
                self.voice = None;
                self.volume = None;
                self.box_key = None;
                self.voice_direct = false;
                self.paired = false;
                self.paired_computers.clear();
                self.wifi_network = None;
                self.wifi_address = None;
                self.wifi_error = None;
                self.wifi_seen.clear();
                self.box_relay = None;
            }
            DeviceMessage::ConnectedNetwork { address } => {
                self.apply(&DeviceMessage::Connected { port: address.clone(), bridge: false, usb_serial: None });
                self.network = true;
                // Reached over Wi-Fi, so that is its address there, even from firmware that doesn't repeat it in answer to hello.
                if let Ok(socket) = address.parse::<std::net::SocketAddr>() {
                    self.wifi_address = Some(socket.ip().to_string());
                }
            }
            DeviceMessage::ConnectedRelay { url } => {
                self.apply(&DeviceMessage::Connected { port: url.clone(), bridge: false, usb_serial: None });
                self.network = true;
                self.relay = true;
            }
            DeviceMessage::Busy { owner, reason } => self.busy = Some(Busy { owner: owner.clone(), reason: reason.clone() }),
            DeviceMessage::ForeignFirmware => {
                // Over the bridge the port survives a reset into other firmware, so what ours reported must go too.
                self.foreign_firmware = true;
                self.unsupported_board = None;
                self.firmware_build = None;
                self.firmware_version = None;
                self.transfer = false;
                self.rolled_back = false;
                self.mode = None;
                self.voice = None;
                self.volume = None;
                self.box_key = None;
                self.voice_direct = false;
                self.paired = false;
                self.paired_computers.clear();
                self.wifi_network = None;
                self.wifi_address = None;
                self.wifi_error = None;
                self.wifi_seen.clear();
                self.box_relay = None;
            }
            DeviceMessage::Disconnected => {
                self.connected = false;
                self.port = None;
                self.network = false;
                self.relay = false;
                self.foreign_firmware = false;
            }
            DeviceMessage::Line(line) => {
                if let Some(mode) = line.strip_prefix("MODE ") {
                    self.mode = Some(mode.trim().to_ascii_lowercase());
                } else if let Some(board) = line.strip_prefix(BOARD_PREFIX) {
                    let board = board.trim();
                    self.unsupported_board = (board != RELEASE_BOARD).then(|| board.to_owned());
                // Anywhere in the line: a line the box wrote before the port was opened can lose its newline and
                // arrive glued in front ("LEISURE SKIT DISPLAY READY BUILD …", seen 2026-10-01), and missing the
                // build hides the firmware update.
                } else if let Some((_, build)) = line.split_once(BUILD_MARKER) {
                    self.foreign_firmware = false;
                    self.firmware_build = Some(build.trim().to_owned());
                    // The box reports these after the build, each time it reports its state.
                    self.transfer = false;
                    self.rolled_back = false;
                } else if line.trim() == "TRANSFER 1" {
                    self.transfer = true;
                } else if line.trim() == "FIRMWARE ROLLED BACK" {
                    self.rolled_back = true;
                } else if let Some(key) = line.strip_prefix(BOX_KEY_PREFIX) {
                    let key = key.trim();
                    // Another key on the same port: the box made itself a new identity and restarted without the
                    // port closing (#122). What the old identity had (pairings, Wi-Fi) is gone with it; the box
                    // reports what it has again.
                    if self.box_key.as_deref().is_some_and(|old| old != key) {
                        self.paired = false;
                        self.paired_computers.clear();
                        self.wifi_network = None;
                        self.wifi_address = None;
                    }
                    self.box_key = Some(key.to_owned());
                } else if line.trim() == "VOICE DIRECT" {
                    self.voice_direct = true;
                } else if line.starts_with("PAIRED ") {
                    self.paired = true;
                } else if let Some(url) = line.strip_prefix("RELAY URL ") {
                    self.box_relay = Some(url.trim().to_owned());
                } else if let Some(network) = line.strip_prefix("WIFI NETWORK ") {
                    // Another network: the address and any failure were the old one's. The same name comes again
                    // with every hello and changes nothing.
                    let network = network.trim();
                    if self.wifi_network.as_deref().is_some_and(|old| old != network) {
                        self.wifi_address = None;
                        self.wifi_error = None;
                    }
                    self.wifi_network = Some(network.to_owned());
                } else if line.trim() == "WIFI FORGOTTEN" {
                    self.wifi_network = None;
                    self.wifi_address = None;
                    self.wifi_error = None;
                } else if line.starts_with("WIFI CONNECTING ") {
                    // Joining again, say with a new password for the same network: the last join's address is gone.
                    self.wifi_address = None;
                } else if let Some(why) = line.strip_prefix("WIFI FAILED ") {
                    self.wifi_address = None;
                    self.wifi_error = Some(wifi_failure(why.trim()));
                } else if line.trim() == "WIFI CONNECTED" {
                    self.wifi_error = None;
                } else if line.trim() == "WIFI DISCONNECTED" {
                    self.wifi_address = None;
                } else if let Some(seen) = line.strip_prefix("WIFI SEEN ") {
                    // `WIFI SEEN <count>` starts a scan; `WIFI SEEN <rssi> <channel> <ssid>` is one network.
                    let mut parts = seen.splitn(3, ' ');
                    match (parts.next().map(str::parse::<i32>), parts.next().map(str::parse::<u8>), parts.next()) {
                        (Some(Ok(_)), None, None) => self.wifi_seen.clear(),
                        (Some(Ok(rssi)), Some(Ok(_)), Some(ssid)) if !ssid.trim().is_empty() => {
                            let ssid = ssid.trim_end().to_owned();
                            match self.wifi_seen.iter_mut().find(|network| network.ssid == ssid) {
                                Some(network) => network.rssi = network.rssi.max(rssi),
                                None => self.wifi_seen.push(SeenNetwork { ssid, rssi }),
                            }
                            self.wifi_seen.sort_by_key(|network| std::cmp::Reverse(network.rssi));
                        }
                        _ => {}
                    }
                } else if let Some(address) = line.strip_prefix("WIFI ADDRESS ") {
                    self.wifi_address = Some(address.trim().to_owned());
                    self.wifi_error = None;
                } else if line.starts_with("PAIRS ") {
                    self.paired_computers.clear();
                } else if let Some((key, name)) = line.strip_prefix("PAIR ").and_then(|rest| rest.split_once(' '))
                    && key.len() == 43
                {
                    self.paired_computers.push(PairedComputer { key: key.to_owned(), name: name.trim().to_owned() });
                } else if line.trim() == "UNPAIRED ALL" {
                    self.paired = false;
                    self.paired_computers.clear();
                } else if let Some(key) = line.strip_prefix("UNPAIRED ") {
                    let key = key.trim();
                    self.paired_computers.retain(|computer| computer.key != key);
                    if self.computer_key.as_deref() == Some(key) {
                        self.paired = false;
                    }
                } else if let Some((_, version)) = line.split_once(VERSION_MARKER) {
                    self.firmware_version = Some(version.trim().to_owned());
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
            DeviceMessage::Candidates(candidates) => self.candidates = candidates.clone(),
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
    pub opencode: Option<DateTime<Local>>,
    pub copilot: Option<DateTime<Local>>,
    pub pi: Option<DateTime<Local>>,
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
    /// The git description alone, for the settings window; `build` adds the app version and the build time.
    pub revision: String,
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
    pub updates: crate::updates::UpdateStatus,
    /// The person's VibeBuddy account (ADR-0018).
    pub account: crate::account::AccountStatus,
}

/// What the apps explain for `WIFI FAILED <why>`: `wrong_password` (the network turned the box away, which a
/// wrong password causes), `not_found`, `security` (no WPA2 or open network by that name), or the box's own `password`
/// / `ssid` (unusable as given). Anything else is passed on as it came. Firmware up to 0.5.0 sends the driver's whole
/// error, `Failed(DisconnectedInfo { .., reason: AuthenticationExpired, .. })`; later firmware just the reason.
pub fn wifi_failure(why: &str) -> String {
    let reason = why.split_once("reason: ").map_or(why, |(_, rest)| rest.split([',', ' ', '}']).next().unwrap_or(rest));
    match reason {
        "AuthenticationExpired" | "AuthenticationFailed" | "FourWayHandshakeTimeout" | "HandshakeTimeout" | "MicFailure"
        | "_802_1xAuthenticationFailed" => "wrong_password",
        "NoAccessPointFound" | "NoAccessPointFoundInRssiThreshold" | "BeaconTimeout" => "not_found",
        "NoAccessPointFoundWithCompatibleSecurity" | "NoAccessPointFoundInAuthmodeThreshold" => "security",
        _ => why,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> DeviceMessage {
        DeviceMessage::Line(text.to_owned())
    }

    #[test]
    fn candidates_wait_for_a_choice_and_go_once_a_box_connects() {
        let mut state = DeviceState::default();
        let candidates = vec![
            Candidate { port: "/dev/cu.usbmodem1101".to_owned(), usb_serial: Some("30:ED:A0:A4:0D:08".to_owned()) },
            Candidate { port: "/dev/cu.usbmodem8401".to_owned(), usb_serial: Some("98:88:E0:06:8B:CC".to_owned()) },
        ];
        assert!(state.apply(&DeviceMessage::Candidates(candidates.clone())));
        assert_eq!(state.candidates, candidates);
        let usb_serial = Some("98:88:E0:06:8B:CC".to_owned());
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.usbmodem8401".to_owned(), bridge: false, usb_serial });
        assert!(state.candidates.is_empty(), "connected: there is nothing left to choose");
    }

    #[test]
    fn a_build_glued_behind_a_stray_line_is_still_read() {
        let mut state = DeviceState::default();
        state.apply(&line("LEISURE SKIT DISPLAY READY BUILD v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
        assert_eq!(state.firmware_build.as_deref(), Some("v0.2.1-38-ga0bffc7 2026-09-30 16:29"));
    }

    #[test]
    fn a_version_glued_behind_a_stray_line_is_still_read() {
        let mut state = DeviceState::default();
        state.apply(&line("LEISURE SKIT FIRMWARE VERSION 0.3.0"));
        assert_eq!(state.firmware_version.as_deref(), Some("0.3.0"));
    }

    #[test]
    fn device_state_is_read_off_the_diagnostic_lines() {
        let mut state = DeviceState::default();
        assert!(state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true, usb_serial: None }));
        assert!(state.apply(&line("DISPLAY READY BUILD 21a8360-dirty 2026-09-16 10:23")));
        assert!(state.apply(&line("FIRMWARE VERSION 0.2.2")));
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
                network: false,
                relay: false,
                busy: None,
                mode: Some("pomodoro".to_owned()),
                firmware_build: Some("21a8360-dirty 2026-09-16 10:23".to_owned()),
                transfer: false,
                rolled_back: false,
                firmware_version: Some("0.2.2".to_owned()),
                voice: Some("builtin".to_owned()),
                volume: Some(65),
                usb_serial: None,
                box_key: None,
                voice_direct: false,
                paired: false,
                computer_key: None,
                paired_computers: Vec::new(),
                wifi_network: None,
                wifi_address: None,
                wifi_error: None,
                relay_server: None,
                box_relay: None,
                wifi_seen: Vec::new(),
                foreign_firmware: false,
                unsupported_board: None,
                pin: None,
                candidates: Vec::new(),
            }
        );
    }

    #[test]
    fn the_box_key_and_pairing_belong_to_the_box_now_connected() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None });
        state.apply(&line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE"));
        state.apply(&line("PAIRED AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI"));
        assert_eq!(state.box_key.as_deref(), Some("1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE"));
        assert!(state.paired);
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.y".to_owned(), bridge: false, usb_serial: None });
        assert_eq!((state.box_key, state.paired), (None, false));
    }

    #[test]
    fn a_scan_lists_each_network_once_strongest_first() {
        let mut state = DeviceState::default();
        for text in ["WIFI SEEN 4", "WIFI SEEN -61 1 ByteTrade Guest", "WIFI SEEN -49 1 ByteTrade", "WIFI SEEN -68 6 ByteTrade", "WIFI SEEN -70 3 "] {
            state.apply(&line(text));
        }
        let seen: Vec<_> = state.wifi_seen.iter().map(|network| (network.ssid.as_str(), network.rssi)).collect();
        assert_eq!(seen, [("ByteTrade", -49), ("ByteTrade Guest", -61)], "hidden networks are left out");
        state.apply(&line("WIFI SEEN 0"));
        assert!(state.wifi_seen.is_empty(), "a new scan starts over");
    }

    #[test]
    fn a_new_identity_drops_what_the_old_one_had() {
        let mut state = DeviceState::default();
        state.apply(&line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE"));
        state.apply(&line("WIFI NETWORK Home"));
        state.apply(&line("WIFI ADDRESS 192.168.1.23"));
        state.apply(&line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE"));
        assert_eq!(state.wifi_network.as_deref(), Some("Home"), "the same key again changes nothing");
        state.apply(&line("BOX KEY pm3f5yT3yi4s2P90TalP1NcUZD3xBTfETdlii-KnvWM"));
        assert_eq!((state.wifi_network, state.wifi_address), (None, None));
    }

    #[test]
    fn joining_a_network_shows_how_it_went() {
        let mut state = DeviceState::default();
        state.apply(&line("WIFI NETWORK Home"));
        state.apply(&line("WIFI ADDRESS 192.168.1.23"));
        state.apply(&line("WIFI NETWORK Office"));
        assert_eq!(state.wifi_address, None, "the old network's address");
        state.apply(&line("WIFI CONNECTING Office"));
        state.apply(&line("WIFI FAILED password"));
        assert_eq!(state.wifi_error.as_deref(), Some("password"));
        state.apply(&line("WIFI NETWORK Office"));
        assert_eq!(state.wifi_error.as_deref(), Some("password"), "a hello repeats the name, nothing changed");
        state.apply(&line("WIFI CONNECTED"));
        assert_eq!(state.wifi_error, None);
        state.apply(&line("WIFI ADDRESS 10.0.0.7"));
        state.apply(&line("WIFI DISCONNECTED"));
        assert_eq!(state.wifi_address, None);
    }

    #[test]
    fn joining_the_same_network_again_forgets_the_last_address() {
        let mut state = DeviceState::default();
        state.apply(&line("WIFI NETWORK Home"));
        state.apply(&line("WIFI ADDRESS 192.168.1.23"));
        state.apply(&line("WIFI NETWORK Home"));
        state.apply(&line("WIFI CONNECTING Home"));
        assert_eq!(state.wifi_address, None, "not joined while it tries again");
        state.apply(&line("WIFI ADDRESS 192.168.1.23"));
        state.apply(&line("WIFI FAILED AuthenticationExpired"));
        assert_eq!((state.wifi_address, state.wifi_error.as_deref()), (None, Some("wrong_password")));
    }

    #[test]
    fn a_box_reached_over_the_local_network_is_joined_there() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::ConnectedNetwork { address: "192.168.1.160:7340".to_owned() });
        state.apply(&line("WIFI NETWORK Home"));
        assert_eq!(state.wifi_address.as_deref(), Some("192.168.1.160"));
    }

    #[test]
    fn a_failed_join_says_what_kind_of_failure() {
        let mut state = DeviceState::default();
        state.apply(&line("WIFI NETWORK Home"));
        state.apply(&line(r#"WIFI FAILED Failed(DisconnectedInfo { ssid: "Home", bssid: [136, 220, 151, 19, 86, 27], reason: AuthenticationExpired, rssi: -59 })"#));
        assert_eq!(state.wifi_error.as_deref(), Some("wrong_password"), "firmware 0.5.0's whole driver error");
        state.apply(&line("WIFI FAILED NoAccessPointFound"));
        assert_eq!(state.wifi_error.as_deref(), Some("not_found"));
        state.apply(&line("WIFI FAILED NoAccessPointFoundWithCompatibleSecurity"));
        assert_eq!(state.wifi_error.as_deref(), Some("security"));
        state.apply(&line("WIFI FAILED FourWayHandshakeTimeout"));
        assert_eq!(state.wifi_error.as_deref(), Some("wrong_password"));
        state.apply(&line("WIFI FAILED ssid"));
        assert_eq!(state.wifi_error.as_deref(), Some("ssid"));
        state.apply(&line("WIFI FAILED AssociationTooMany"));
        assert_eq!(state.wifi_error.as_deref(), Some("AssociationTooMany"), "passed on as it came");
    }

    #[test]
    fn the_box_says_which_relay_it_uses() {
        let mut state = DeviceState::default();
        state.apply(&line("RELAY URL wss://relay-staging.korekore.ai"));
        assert_eq!(state.box_relay.as_deref(), Some("wss://relay-staging.korekore.ai"));
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None });
        assert_eq!(state.box_relay, None, "a box that comes back says it again");
    }

    #[test]
    fn the_wifi_network_follows_the_box() {
        let mut state = DeviceState::default();
        state.apply(&line("WIFI NETWORK Home 5G"));
        state.apply(&line("WIFI ADDRESS 192.168.1.23"));
        assert_eq!((state.wifi_network.as_deref(), state.wifi_address.as_deref()), (Some("Home 5G"), Some("192.168.1.23")));
        state.apply(&line("WIFI FORGOTTEN"));
        assert_eq!((state.wifi_network, state.wifi_address), (None, None));
    }

    #[test]
    fn busy_lasts_until_a_link_comes_up() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Busy { owner: "omarchy".to_owned(), reason: "linked".to_owned() });
        assert_eq!(state.busy.as_ref().map(|busy| busy.owner.as_str()), Some("omarchy"));
        state.apply(&DeviceMessage::ConnectedRelay { url: "wss://relay.korekore.ai".to_owned() });
        assert!(state.busy.is_none() && state.relay && state.network);
        state.apply(&DeviceMessage::Disconnected);
        assert!(!state.relay && !state.network, "a dropped link isn't through anything");
    }

    #[test]
    fn a_network_link_is_a_connection_with_an_address() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::ConnectedNetwork { address: "192.168.1.23:7340".to_owned() });
        assert!(state.connected && state.network);
        assert_eq!(state.port.as_deref(), Some("192.168.1.23:7340"));
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None });
        assert!(state.connected && !state.network, "back on the cable, with no drop in between");
    }

    #[test]
    fn the_paired_list_follows_what_the_box_reports() {
        let mine = "A".repeat(43);
        let other = "B".repeat(43);
        let mut state = DeviceState { computer_key: Some(mine.clone()), ..DeviceState::default() };
        state.apply(&line(&format!("PAIRED {mine}")));
        state.apply(&line("PAIRS 2"));
        state.apply(&line(&format!("PAIR {mine} dragon's MacBook")));
        state.apply(&line(&format!("PAIR {other} omarchy")));
        assert_eq!(state.paired_computers.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["dragon's MacBook", "omarchy"]);
        assert!(!state.apply(&line("PAIR FULL")), "not a list entry");
        state.apply(&line(&format!("UNPAIRED {other}")));
        assert_eq!(state.paired_computers.len(), 1);
        assert!(state.paired);
        state.apply(&line("UNPAIRED ALL"));
        assert!(state.paired_computers.is_empty());
        assert!(!state.paired);
    }

    #[test]
    fn a_board_other_than_the_box_is_flagged_until_another_device_connects() {
        let mut state = DeviceState::default();
        state.apply(&line("BOARD alientek-box"));
        assert_eq!(state.unsupported_board, None, "a build for the box itself");
        state.apply(&line("BOARD goouuu-s3-spi"));
        assert_eq!(state.unsupported_board.as_deref(), Some("goouuu-s3-spi"));
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None });
        assert_eq!(state.unsupported_board, None, "released firmware says no board: the box");
    }

    #[test]
    fn disconnecting_keeps_the_last_known_firmware_and_voice() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true, usb_serial: None });
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
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true, usb_serial: None });
        state.apply(&line("DISPLAY READY BUILD abc 2026-09-16 10:23"));
        state.apply(&line("FIRMWARE VERSION 0.2.2"));
        state.apply(&DeviceMessage::Disconnected);
        assert!(state.apply(&DeviceMessage::Connected { port: "/dev/cu.y".to_owned(), bridge: false, usb_serial: None }));
        assert_eq!(state.firmware_version, None);
        assert_eq!(state.firmware_build, None);
    }

    #[test]
    fn a_new_connection_forgets_everything_the_previous_device_said() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None });
        for said in ["MODE DUTY", "VOICES ahu", "VOLUME 50"] {
            state.apply(&line(said));
        }
        state.apply(&DeviceMessage::Disconnected);
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.y".to_owned(), bridge: false, usb_serial: None });
        assert_eq!((state.mode.as_deref(), state.voice.as_deref(), state.volume), (None, None, None));
    }

    #[test]
    fn a_device_without_the_voices_partition_reports_no_voice() {
        let mut state = DeviceState::default();
        assert!(!state.apply(&line("VOICES NO PARTITION")));
        assert_eq!(state.voice, None);
    }

    #[test]
    fn a_box_running_other_firmware_is_there_until_it_reports_our_build() {
        let mut state = DeviceState::default();
        state.apply(&DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: true, usb_serial: None });
        state.apply(&line("DISPLAY READY BUILD abc 2026-09-16 10:23"));
        state.apply(&line("VOICES xiaohe2"));
        // Switched to Muse over the bridge: the port stayed open, so what our firmware said is on record.
        assert!(state.apply(&DeviceMessage::ForeignFirmware));
        assert!(state.connected, "a box running other firmware is there, not offline");
        assert!(state.foreign_firmware);
        assert_eq!((state.firmware_build.as_deref(), state.voice.as_deref()), (None, None));
        assert_eq!(serde_json::to_value(&state).unwrap()["foreign_firmware"], true, "/v1/status carries it");

        assert!(state.apply(&line("DISPLAY READY BUILD def 2026-10-07 09:00")));
        assert!(!state.foreign_firmware);
        state.apply(&DeviceMessage::ForeignFirmware);
        state.apply(&DeviceMessage::Disconnected);
        assert!(!state.foreign_firmware, "a box that's gone runs nothing");
    }
}