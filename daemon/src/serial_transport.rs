use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use vibebuddy_protocol::Event;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_serial::{ClearBuffer, SerialPort, SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::{debug, info, warn};

use crate::lan::{self, LanConfig, LinkError};
use crate::relay::{self, RelayConfig, RelayError};

const ESPRESSIF_VID: u16 = 0x303a;
const USB_SERIAL_JTAG_PID: u16 = 0x1001;
const QINHENG_VID: u16 = 0x1a86;
const USB_SINGLE_SERIAL_PID: u16 = 0x55d3;
const QUEUE_CAPACITY: usize = 64;
/// A screenshot is 240 lines and a voice-pack write gets one receipt per block; the queue must hold a full screen.
const DEVICE_EVENT_CAPACITY: usize = 512;
/// The rate every link over the bridge starts at, the only one firmware older than the rate switch knows, and the
/// ROM bootloader's.
const BAUD_RATE: u32 = 115_200;
/// The rate the bridge moves to once the box agrees (docs/protocol.md, "Link speed on the UART bridge"): about 92 KB/s
/// against the 16 KB/s of a voice command's audio. The box's CH343 bridge and its UART both run it.
pub const FAST_BAUD_RATE: u32 = 921_600;
/// How long the box has to answer `device.baud` before it counts as firmware that can't switch, and then to confirm
/// the new rate; the box itself gives a switch 2 seconds.
const BAUD_ANSWER_WITHIN: Duration = Duration::from_secs(3);
/// After moving the port to a new rate, before writing at it: the box moves as soon as its line is out.
const BAUD_SETTLE: Duration = Duration::from_millis(20);
/// Tries at the fast rate per boot of the box: a bridge that can't hold it stays at 115200.
const BAUD_TRIES: u32 = 3;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
const CONNECT_SETTLE_DELAY: Duration = Duration::from_millis(1_500);
/// While linked over the network, how often to look for the box on the cable.
const CABLE_CHECK_EVERY: Duration = Duration::from_secs(2);
/// While linked through the Relay, how often to see whether the box answers on the local network.
const LAN_CHECK_EVERY: Duration = Duration::from_secs(30);
/// After the box answered busy, how long before asking it again over the network. Each try costs the box a Noise
/// handshake, about 170 ms of its main loop; tries every half second made its speaker click.
const BUSY_RETRY: Duration = Duration::from_secs(15);
/// The BOX's CH343 UART bridge can't swallow a whole line: push more than one or two hundred bytes at once and
/// its buffer gets flushed, dropping 32 bytes every 384 and repeating the next 32, so the length stays right but
/// the content shifts. A control experiment polling the FIFO directly on the device cleared the device side. Over the bridge,
/// write in line-rate chunks and wait for each to clear the wire before writing the next.
const PACE_MARGIN: Duration = Duration::from_millis(1);
/// A box just connected or just reset is listened to this long before anything is written to it. Our firmware
/// reports its build at boot, about two seconds after a reset; other ESP-IDF firmware logs within that time, and
/// Muse's heartbeat line comes every five seconds even when nothing else happens.
const LISTEN_FIRST: Duration = Duration::from_secs(6);
/// After hello, a box that still hasn't reported a build runs other firmware.
const FOREIGN_FIRMWARE_AFTER: Duration = Duration::from_secs(5);
/// Every line of our firmware's build report, at boot and in answer to hello, carries this; it can arrive glued
/// behind a stray line.
pub const BUILD_MARKER: &str = "DISPLAY READY BUILD ";
/// The ESP32-S3 ROM prints this at every reset, on the native USB port and on the UART bridge alike. A box that
/// resets may come back running other firmware (K1 + K2 on a box shared with Muse), so it is judged again.
const ROM_BANNER: &str = "ESP-ROM:";

/// Everything from the device to the Mac: JSON events, diagnostic lines, and the link connecting and dropping.
#[derive(Clone, Debug)]
pub enum DeviceMessage {
    Event(Event),
    Line(String),
    /// `bridge` means we're on the BOX's CH343 UART bridge: writes must be chunked and flashing uses small blocks.
    /// `usb_serial` is the port's USB serial number, when the system knows it.
    Connected { port: String, bridge: bool, usb_serial: Option<String> },
    /// Linked over the local network, to the box at this address (ADR-0012). Moving between this and the cable
    /// is not a drop: no Disconnected comes in between.
    ConnectedNetwork { address: String },
    /// Linked through the Relay at this address (ADR-0012); like ConnectedNetwork, moving to or from it is no drop.
    ConnectedRelay { url: String },
    /// The box is linked to another computer, by name when known; `reason` is `usb` or `linked`. Sent when it
    /// changes, while this computer isn't linked.
    Busy { owner: String, reason: String },
    /// The box runs other firmware (see [`Firmware::Foreign`]): it's there, but gets nothing written.
    ForeignFirmware,
    /// Several devices could be the box and none is the one remembered, so none is connected until the user picks
    /// one; empty once that is no longer so.
    Candidates(Vec<Candidate>),
    /// The UART bridge's rate is settled for now: [`FAST_BAUD_RATE`] carries voice commands' audio, 115200 doesn't.
    /// `switches` says whether the firmware answered the request to move (firmware before the switch doesn't).
    BridgeRate { baud: u32, switches: bool },
    Disconnected,
}

/// A device that could be the box, offered to the user when there are several.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Candidate {
    pub port: String,
    /// What a choice is remembered by; without one the device can't be chosen.
    pub usb_serial: Option<String>,
}

/// An environment variable that narrows the daemon to one port or device. Meant for development; left set, it
/// hides every other box behind "Box not found", so the status reports it (2026-10-08: a pin set for the Muse
/// tests outlived them and a swapped box looked unfound).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Pin {
    pub variable: &'static str,
    pub value: String,
}

pub trait Transport: Send + Sync {
    fn send(&self, frame: Vec<u8>) -> Result<(), TransportError>;
}

#[derive(Clone, Default)]
pub struct SerialConfig {
    explicit_port: Option<String>,
    usb_serial: Option<String>,
    /// Where the USB serial number of the last device that proved to be the box is kept; see [`remember_box`].
    known_box: Option<PathBuf>,
    /// How to reach the box without the cable; None when this computer has no pairing key.
    lan: Option<Arc<LanConfig>>,
    /// The Relay, tried when neither the cable nor the local network reaches the box.
    relay: Option<RelayConfig>,
    /// The user asked to take the box from whoever has it: the next network or Relay attempt says takeover.
    takeover: Arc<AtomicBool>,
    /// The user's Relay switch (config `relay`); off, the Relay isn't tried and a link through it ends.
    relay_enabled: Arc<AtomicBool>,
}

impl SerialConfig {
    pub fn from_env(known_box: Option<PathBuf>) -> Self {
        Self {
            explicit_port: env::var("VIBEBUDDY_SERIAL_PORT").ok(),
            usb_serial: env::var("VIBEBUDDY_USB_SERIAL").ok(),
            known_box,
            lan: None,
            relay: None,
            takeover: Arc::new(AtomicBool::new(false)),
            relay_enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Lets the worker reach the box through the Relay when nothing closer does.
    pub fn with_relay(mut self, relay: RelayConfig) -> Self {
        self.relay = Some(relay);
        self
    }

    /// Lets the worker fall back to the local network when no box is on the cable.
    pub fn with_lan(mut self, lan: LanConfig) -> Self {
        self.lan = Some(Arc::new(lan));
        self
    }

    pub fn pin(&self) -> Option<Pin> {
        match (&self.explicit_port, &self.usb_serial) {
            (Some(port), _) => Some(Pin { variable: "VIBEBUDDY_SERIAL_PORT", value: port.clone() }),
            (None, Some(serial)) => Some(Pin { variable: "VIBEBUDDY_USB_SERIAL", value: serial.clone() }),
            (None, None) => None,
        }
    }

    fn known_box_serial(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.known_box.as_ref()?).ok()?;
        let serial = text.trim();
        (!serial.is_empty()).then(|| serial.to_owned())
    }
}

/// Remembers a device as the box once it has reported our firmware: every ESP32-S3 on native USB is
/// the same 303A:1001, so with another one plugged in (a Muse, a devkit) the serial number is the only
/// way to tell which is ours.
pub fn remember_box(path: &std::path::Path, usb_serial: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{usb_serial}\n"));
}

#[derive(Debug, PartialEq)]
pub enum TransportError {
    Closed,
    QueueFull,
}

pub struct SerialTransport {
    sender: mpsc::Sender<Vec<u8>>,
    suspend: watch::Sender<bool>,
    takeover: Arc<AtomicBool>,
    relay_enabled: Arc<AtomicBool>,
}

impl SerialTransport {
    pub fn spawn(config: SerialConfig) -> (Self, mpsc::Receiver<DeviceMessage>) {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (device_sender, device_receiver) = mpsc::channel(DEVICE_EVENT_CAPACITY);
        let (suspend, suspend_receiver) = watch::channel(false);
        let takeover = config.takeover.clone();
        let relay_enabled = config.relay_enabled.clone();
        tokio::spawn(serial_worker(config, receiver, device_sender, suspend_receiver));
        (Self { sender, suspend, takeover, relay_enabled }, device_receiver)
    }

    /// The user's Relay switch; turned off, a link through the Relay ends within a couple of seconds.
    pub fn set_relay_enabled(&self, enabled: bool) {
        self.relay_enabled.store(enabled, Ordering::Relaxed);
    }

    /// The user chose to take the box from the computer that has it (ADR-0011).
    pub fn request_takeover(&self) {
        self.takeover.store(true, Ordering::Relaxed);
    }

    /// Hands the serial port to the flasher: the worker closes the port and stops reconnecting until released.
    pub fn set_suspended(&self, suspended: bool) {
        let _ = self.suspend.send(suspended);
    }
}

impl Transport for SerialTransport {
    fn send(&self, frame: Vec<u8>) -> Result<(), TransportError> {
        self.sender.try_send(frame).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => TransportError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })
    }
}

async fn serial_worker(
    config: SerialConfig,
    mut receiver: mpsc::Receiver<Vec<u8>>,
    device_event_sender: mpsc::Sender<DeviceMessage>,
    mut suspend: watch::Receiver<bool>,
) {
    let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
    // What the status last heard about candidates, so a choice that stays open is reported (and logged) once.
    let mut offered: Vec<Candidate> = Vec::new();
    // A Connected went out and no Disconnected since. A session that ends only says Disconnected once neither the
    // cable nor the network brings the box back, so moving from one to the other isn't a drop.
    let mut linked = false;
    let mut busy_logged: Option<String> = None;
    // When the Relay may be tried again, and how many tries in a row have failed.
    let mut relay_retry: Option<Instant> = None;
    let mut relay_attempts = 0_u32;
    // When the box, which answered busy, may be asked again over the network.
    let mut busy_retry: Option<Instant> = None;

    loop {
        if *suspend.borrow() {
            // While flashing, leave the port alone and don't queue frames: heartbeats keep coming, but they're stale once the device reboots.
            pending.clear();
            tokio::select! {
                changed = suspend.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                frame = receiver.recv() => {
                    if frame.is_none() {
                        return;
                    }
                }
            }
            continue;
        }
        let found = find_port(&config);
        let candidates = match &found {
            Ok(Found::Several(choices)) => choices
                .iter()
                .map(|choice| Candidate { port: choice.name.clone(), usb_serial: choice.usb_serial.clone() })
                .collect(),
            _ => Vec::new(),
        };
        if candidates != offered {
            if !candidates.is_empty() {
                let ports = candidates.iter().map(|candidate| candidate.port.as_str()).collect::<Vec<_>>().join(", ");
                warn!(%ports, "several devices could be the box and none is the known one; waiting for a choice");
            }
            let _ = device_event_sender.send(DeviceMessage::Candidates(candidates.clone())).await;
            offered = candidates;
        }
        let choice = match found {
            Ok(Found::One(choice)) => Some(choice),
            Ok(Found::None | Found::Several(_)) => None,
            Err(error) => {
                warn!(%error, "serial port discovery failed, retrying later");
                None
            }
        };
        let Some(PortChoice { name: port_name, paced, usb_serial }) = choice else {
            // No box on the cable: try the local network, then the Relay, if this computer is paired with a box.
            // Taken here only for the local network; the Relay takes it on its socket, even mid-wait.
            let takeover = config.takeover.load(Ordering::Relaxed);
            let leave_alone = leave_busy_box_alone(busy_retry, Instant::now(), takeover);
            let mut busy = None;
            if !leave_alone
                && let Some(lan) = &config.lan
                && let Some((target, result)) = lan::link(lan, takeover).await
            {
                match result {
                    Ok(mut stream) => {
                        config.takeover.store(false, Ordering::Relaxed);
                        info!(address = %target.address, "linked over the local network");
                        busy_logged = None;
                        busy_retry = None;
                        let _ = device_event_sender.send(DeviceMessage::ConnectedNetwork { address: target.address.clone() }).await;
                        linked = true;
                        let end = run_session(
                            &mut stream,
                            &target.address,
                            false,
                            None,
                            Firmware::Ours,
                            Some(&config),
                            None,
                            None,
                            &mut pending,
                            &mut receiver,
                            &device_event_sender,
                            &mut suspend,
                        )
                        .await;
                        match end {
                            SessionEnd::Shutdown => return,
                            SessionEnd::Usb => info!("the box is on the cable, leaving the local network link"),
                            _ => {}
                        }
                        continue;
                    }
                    Err(LinkError::Busy { reason, owner }) => {
                        busy = Some((owner, reason));
                        busy_retry = Some(Instant::now() + BUSY_RETRY);
                    }
                    Err(error) => debug!(?error, address = %target.address, "no local network link"),
                }
            }
            // A box that answered busy on the local network is busy on the Relay too.
            let box_key = config.lan.as_ref().and_then(|lan| lan.box_key());
            if busy.is_none()
                && !leave_alone
                && config.relay_enabled.load(Ordering::Relaxed)
                && let (Some(relay_config), Some(box_key)) = (&config.relay, box_key)
                && !relay_config.waits_for_account()
                && (takeover || relay_retry.is_none_or(|at| Instant::now() >= at))
            {
                let reported_busy = std::sync::Mutex::new(busy_logged.clone());
                let mut on_busy = |owner: String, reason: String| {
                    let mut reported = reported_busy.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if reported.as_ref() != Some(&format!("{owner}/{reason}")) {
                        info!(%owner, %reason, "the box is linked elsewhere; waiting on the Relay");
                        *reported = Some(format!("{owner}/{reason}"));
                        let _ = device_event_sender.try_send(DeviceMessage::Busy { owner, reason });
                    }
                };
                // Waiting on the socket ends when the box turns up on the cable or the user turns the Relay off.
                let give_up = || matches!(find_port(&config), Ok(Found::One(_))) || !config.relay_enabled.load(Ordering::Relaxed);
                let events = device_event_sender.clone();
                let url = relay_config.url();
                let presence: Arc<dyn Fn(bool) + Send + Sync> = Arc::new(move |here| {
                    let message = if here { DeviceMessage::ConnectedRelay { url: url.clone() } } else { DeviceMessage::Disconnected };
                    info!(here, "the box's presence on the Relay changed");
                    let _ = events.try_send(message);
                });
                let memory = config.lan.as_ref().map(|lan| lan.memory());
                let wait = relay::Wait { takeover: &config.takeover, busy: &mut on_busy, give_up: &give_up, presence, memory };
                let relay_link = relay::link(relay_config, &box_key, wait).await;
                busy_logged = reported_busy.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
                match relay_link {
                    Ok(mut stream) => {
                        let url = relay_config.url();
                        info!(%url, "linked through the Relay");
                        busy_logged = None;
                        busy_retry = None;
                        relay_retry = None;
                        let linked_at = Instant::now();
                        let _ = device_event_sender.send(DeviceMessage::ConnectedRelay { url: url.clone() }).await;
                        linked = true;
                        let end = run_session(
                            &mut stream,
                            &url,
                            false,
                            None,
                            Firmware::Ours,
                            Some(&config),
                            config.lan.as_deref(),
                            Some(&config.relay_enabled),
                            &mut pending,
                            &mut receiver,
                            &device_event_sender,
                            &mut suspend,
                        )
                        .await;
                        match end {
                            SessionEnd::Shutdown => return,
                            SessionEnd::Usb => info!("the box is on the cable, leaving the Relay"),
                            SessionEnd::Lan => info!("the box is on the local network, leaving the Relay"),
                            _ => {
                                // The Relay counts new connections per IP (docs/relay-protocol.md): a link that keeps
                                // ending right after it starts must not become a tight reconnect loop.
                                let (wait, attempts) = relay::after_link(linked_at.elapsed(), relay_attempts);
                                relay_retry = Some(Instant::now() + wait);
                                relay_attempts = attempts;
                            }
                        }
                        continue;
                    }
                    Err(RelayError::Account(code)) => {
                        info!(%code, "the Relay turned this computer away; not trying again until the account or its plan changes");
                        relay_retry = None;
                        relay_attempts = 0;
                    }
                    Err(RelayError::Upgrade) => {
                        warn!("the Relay no longer speaks this version of VibeBuddy; update to reach the box through it");
                        relay_retry = Some(Instant::now() + RelayError::Upgrade.backoff(0));
                    }
                    Err(error) => {
                        debug!(?error, "no Relay link");
                        relay_retry = Some(Instant::now() + error.backoff(relay_attempts));
                        relay_attempts = relay_attempts.saturating_add(1);
                    }
                }
            }
            if let Some((owner, reason)) = busy
                && busy_logged.as_ref() != Some(&format!("{owner}/{reason}"))
            {
                info!(%owner, %reason, "the box is linked elsewhere");
                busy_logged = Some(format!("{owner}/{reason}"));
                let _ = device_event_sender.send(DeviceMessage::Busy { owner, reason }).await;
            }
            if linked {
                let _ = device_event_sender.send(DeviceMessage::Disconnected).await;
                linked = false;
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
            continue;
        };

        let mut port = match open_port(&port_name) {
            Ok(port) => port,
            Err(error) => {
                warn!(port = %port_name, %error, "failed to open serial port, retrying later");
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };
        info!(port = %port_name, paced, "serial port connected");
        tokio::time::sleep(CONNECT_SETTLE_DELAY).await;
        let _ = device_event_sender
            .send(DeviceMessage::Connected { port: port_name.clone(), bridge: paced, usb_serial })
            .await;
        linked = true;

        let end = run_session(
            &mut port,
            &port_name,
            paced,
            paced.then_some(set_port_rate as RateSetter<SerialStream>),
            Firmware::Listening { since: Instant::now() },
            None,
            None,
            None,
            &mut pending,
            &mut receiver,
            &device_event_sender,
            &mut suspend,
        )
        .await;
        match end {
            SessionEnd::Shutdown => return,
            SessionEnd::Reconnect | SessionEnd::Usb | SessionEnd::Lan => {}
            // Closing a tty waits for unsent output to drain, which never happens if the peer doesn't read.
            SessionEnd::Released => drop(port.clear(ClearBuffer::Output)),
        }

        drop(port);
        // Flashing takes the box away for real; otherwise the next look around decides whether it's gone.
        if end == SessionEnd::Released {
            let _ = device_event_sender.send(DeviceMessage::Disconnected).await;
            linked = false;
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// How one connected session ended.
#[derive(Debug, PartialEq)]
enum SessionEnd {
    Reconnect,
    /// The flasher asked for the port.
    Released,
    /// The daemon is shutting down.
    Shutdown,
    /// A box turned up on the cable while linked over the network.
    Usb,
    /// The box answers on the local network while linked through the Relay.
    Lan,
}

/// What the box on the other end runs, judged from what it prints. No port is opened just to ask, since opening
/// the native USB port may reset the chip, and nothing that could act as input is written until it is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Firmware {
    /// Just connected or just reset: nothing is written while the box is listened to.
    Listening { since: Instant },
    /// It said nothing that gives it away: hello alone goes out, and the build should come back.
    Asked { since: Instant },
    Ours,
    /// Other firmware. Its console may read our JSON as key presses (Muse's did: 'a'/'s' menu, 'd'/'u'
    /// push-to-talk, 'z'/'w' sleep), so it is written nothing at all, not even hello, until it resets; and its
    /// output doesn't pass for ours.
    Foreign,
}

impl Firmware {
    fn after_line(self, line: &str, now: Instant) -> Self {
        if line.contains(ROM_BANNER) {
            Self::Listening { since: now }
        } else if line.contains(BUILD_MARKER) {
            Self::Ours
        } else if self != Self::Ours && rate_line(line).starts_with(BAUD_PREFIX) {
            // Only our firmware says this: it went back to 115200 because what it heard was at another rate (the
            // port reopened while it ran fast), so hello was lost. Listened to enough: ask it again now.
            Self::Listening { since: now.checked_sub(LISTEN_FIRST).unwrap_or(now) }
        } else if self != Self::Ours && is_esp_idf_app_log(line) {
            Self::Foreign
        } else {
            self
        }
    }

    /// When this state ends on its own, if it does.
    fn deadline(self) -> Option<Instant> {
        match self {
            Self::Listening { since } => Some(since + LISTEN_FIRST),
            Self::Asked { since } => Some(since + FOREIGN_FIRMWARE_AFTER),
            Self::Ours | Self::Foreign => None,
        }
    }

    /// Whether this frame may be written now.
    fn writes(self, frame: &[u8]) -> bool {
        match self {
            Self::Ours => true,
            Self::Asked { .. } => is_hello(frame),
            Self::Listening { .. } | Self::Foreign => false,
        }
    }
}

/// An ESP-IDF application's log line, `I (1234) tag: ...`. Our firmware never prints one; the second-stage
/// bootloader both firmwares boot through does, under its own tags, so those don't count.
fn is_esp_idf_app_log(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(['I', 'W', 'E', 'D', 'V']).and_then(|rest| rest.strip_prefix(" (")) else {
        return false;
    };
    let Some((ticks, rest)) = rest.split_once(") ") else {
        return false;
    };
    let Some((tag, _)) = rest.split_once(':') else {
        return false;
    };
    !ticks.is_empty()
        && ticks.bytes().all(|byte| byte.is_ascii_digit())
        && !tag.is_empty()
        && !tag.contains(' ')
        && !tag.starts_with("boot")
        && tag != "esp_image"
}

/// The box's lines about the bridge's rate: `BAUD SWITCH <rate>` and `BAUD <rate>`.
const BAUD_PREFIX: &str = "BAUD ";
/// What firmware without the rate switch says to `device.baud`, as to any event it doesn't know.
const BAUD_UNKNOWN: &str = "EVENT device.baud";

/// A line as the box said it about the rate: without what came before it at the other rate. The box's last bytes at
/// 921600, read at 115200, are not ASCII text, and they run into its `BAUD 115200` (seen on a box: `��BAUD 115200`).
fn rate_line(line: &str) -> &str {
    line.trim_start_matches(|c: char| !c.is_ascii_graphic()).trim_end()
}

/// Moves a link to another rate; only the serial port over the bridge has one.
type RateSetter<P> = fn(&mut P, u32) -> std::io::Result<()>;

fn set_port_rate(port: &mut SerialStream, baud: u32) -> std::io::Result<()> {
    port.set_baud_rate(baud).map_err(std::io::Error::from)
}

/// Where the bridge's rate stands with the box (docs/protocol.md, "Link speed on the UART bridge").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RateStage {
    /// Not asked since the box booted, or asked again after the fast rate failed.
    Unasked,
    /// `device.baud` went out; waiting for `BAUD SWITCH`.
    Asked { since: Instant },
    /// Both ends moved; the check went out at the new rate, waiting for `BAUD <rate>`.
    Checking { since: Instant },
    /// Nothing more to do until the box restarts.
    Settled,
}

/// What the session does about the rate next.
#[derive(Debug, PartialEq, Eq)]
enum RateStep {
    Write(Vec<u8>),
    /// Move the port; frames written after it wait a moment for the box to move too.
    SetRate(u32),
    /// Tell the daemon: [`DeviceMessage::BridgeRate`].
    Settled { baud: u32, switches: bool },
}

/// Moves the UART bridge to [`FAST_BAUD_RATE`] once the box is known to run our firmware, and back to 115200 when the
/// ends may no longer agree. Firmware that can't switch, or a switch that isn't confirmed, leaves the link at 115200.
#[derive(Debug)]
struct BaudSwitch {
    /// The port's rate.
    rate: u32,
    stage: RateStage,
    /// Tries since the box booted.
    tries: u32,
    /// The firmware answered `device.baud`.
    switches: bool,
}

impl BaudSwitch {
    fn new() -> Self {
        Self { rate: BAUD_RATE, stage: RateStage::Unasked, tries: 0, switches: false }
    }

    /// Frames from the daemon wait while the rate is being changed: one written at the old rate after the box moved
    /// would arrive as noise.
    fn holds_writes(&self) -> bool {
        matches!(self.stage, RateStage::Asked { .. } | RateStage::Checking { .. })
    }

    fn deadline(&self) -> Option<Instant> {
        match self.stage {
            RateStage::Asked { since } | RateStage::Checking { since } => Some(since + BAUD_ANSWER_WITHIN),
            RateStage::Unasked | RateStage::Settled => None,
        }
    }

    /// The box was just found running our firmware: ask it to move, unless it has been asked enough.
    fn on_ours(&mut self, now: Instant) -> Vec<RateStep> {
        if self.stage != RateStage::Unasked || self.rate != BAUD_RATE || self.tries >= BAUD_TRIES {
            return Vec::new();
        }
        self.tries += 1;
        self.stage = RateStage::Asked { since: now };
        let mut request = Event::named("device.baud");
        request.extra.insert("rate".to_owned(), FAST_BAUD_RATE.into());
        vec![RateStep::Write(request.to_ndjson().expect("the request always encodes"))]
    }

    /// The box restarted (its ROM banner, readable only at 115200): it may be asked again.
    fn on_restart(&mut self) {
        self.stage = RateStage::Unasked;
        self.tries = 0;
    }

    /// A line from the box about the rate.
    fn on_line(&mut self, line: &str, now: Instant) -> Vec<RateStep> {
        let line = line.trim();
        match self.stage {
            RateStage::Asked { .. } if line == format!("BAUD SWITCH {FAST_BAUD_RATE}") => {
                self.switches = true;
                self.rate = FAST_BAUD_RATE;
                self.stage = RateStage::Checking { since: now };
                let check = Event::named("device.baud.check").to_ndjson().expect("the check always encodes");
                vec![RateStep::SetRate(FAST_BAUD_RATE), RateStep::Write(check)]
            }
            RateStage::Asked { .. } if line == BAUD_UNKNOWN => {
                self.stage = RateStage::Settled;
                vec![RateStep::Settled { baud: BAUD_RATE, switches: false }]
            }
            RateStage::Checking { .. } if line == format!("BAUD {FAST_BAUD_RATE}") => {
                self.stage = RateStage::Settled;
                vec![RateStep::Settled { baud: FAST_BAUD_RATE, switches: true }]
            }
            // The box going back on its own, said at the fast rate before a restart.
            _ if self.rate != BAUD_RATE && line == format!("BAUD {BAUD_RATE}") => self.back_to_slow(),
            _ => Vec::new(),
        }
    }

    /// What came over the bridge can't be our firmware at this rate: the box is at 115200 again (it restarted, or
    /// gave up on the switch).
    fn on_garbled(&mut self) -> Vec<RateStep> {
        if self.rate == BAUD_RATE {
            return Vec::new();
        }
        self.back_to_slow()
    }

    fn on_deadline(&mut self) -> Vec<RateStep> {
        match self.stage {
            // No answer at all: firmware that can't switch, which said nothing we knew, or the request was lost.
            RateStage::Asked { .. } => {
                self.stage = RateStage::Settled;
                vec![RateStep::Settled { baud: BAUD_RATE, switches: self.switches }]
            }
            // Not confirmed: the box goes back by itself too.
            RateStage::Checking { .. } => self.back_to_slow(),
            RateStage::Unasked | RateStage::Settled => Vec::new(),
        }
    }

    fn back_to_slow(&mut self) -> Vec<RateStep> {
        self.rate = BAUD_RATE;
        self.stage = RateStage::Unasked;
        vec![RateStep::SetRate(BAUD_RATE), RateStep::Settled { baud: BAUD_RATE, switches: self.switches }]
    }
}

/// Bytes that never come from our firmware at the rate the port runs: NUL, or bytes no UTF-8 text holds. A box at
/// 115200 read at 921600 gives little else.
fn garbled(bytes: &[u8]) -> bool {
    bytes.iter().any(|&byte| byte == 0 || byte >= 0xf5)
}

/// Carries out what [`BaudSwitch`] decided.
async fn take_rate_steps<P: AsyncWrite + Unpin>(
    port: &mut P,
    set_rate: Option<RateSetter<P>>,
    steps: Vec<RateStep>,
    paced: bool,
    device_event_sender: &mpsc::Sender<DeviceMessage>,
) -> std::io::Result<()> {
    for step in steps {
        match step {
            // Short frames on either side of a switch: paced at 115200, which is never too fast.
            RateStep::Write(frame) => write_frame(port, &frame, paced.then_some(BAUD_RATE)).await?,
            RateStep::SetRate(baud) => {
                if let Some(set_rate) = set_rate {
                    set_rate(port, baud)?;
                }
                tokio::time::sleep(BAUD_SETTLE).await;
            }
            RateStep::Settled { baud, switches } => {
                info!(baud, switches, "UART bridge rate settled");
                let _ = device_event_sender.send(DeviceMessage::BridgeRate { baud, switches }).await;
            }
        }
    }
    Ok(())
}

fn hello_frame() -> Vec<u8> {
    Event::named("device.hello").to_ndjson().expect("hello always encodes")
}

fn is_hello(frame: &[u8]) -> bool {
    serde_json::from_slice::<Event>(frame).is_ok_and(|event| event.event == "device.hello")
}

/// Shuttles frames and device output over one open port until it fails or is handed to the flasher.
#[allow(clippy::too_many_arguments, reason = "one session's whole context; a struct would only rename the list")]
async fn run_session<P: AsyncRead + AsyncWrite + Unpin>(
    port: &mut P,
    port_name: &str,
    paced: bool,
    set_rate: Option<RateSetter<P>>,
    mut firmware: Firmware,
    watch_cable: Option<&SerialConfig>,
    probe_lan: Option<&LanConfig>,
    relay_switch: Option<&AtomicBool>,
    pending: &mut VecDeque<Vec<u8>>,
    receiver: &mut mpsc::Receiver<Vec<u8>>,
    device_event_sender: &mpsc::Sender<DeviceMessage>,
    suspend: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut read_buffer = [0_u8; 256];
    let mut line_buffer = Vec::new();
    // Over the network the cable wins: look for the box on it now and then.
    let mut cable_check = tokio::time::interval(CABLE_CHECK_EVERY);
    cable_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Through the Relay, the local network wins when the box answers there.
    let mut lan_check = tokio::time::interval_at(Instant::now() + LAN_CHECK_EVERY, LAN_CHECK_EVERY);
    lan_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Only the bridge has a rate to change.
    let mut baud = BaudSwitch::new();

    loop {
        // Frames wait, in order, until the box is known to run our firmware; only hello may go first.
        let next = if baud.holds_writes() {
            None
        } else {
            pending.iter().position(|frame| firmware.writes(frame)).and_then(|index| pending.remove(index))
        };
        if let Some(frame) = next {
            // A peer that never reads (e.g. firmware that isn't ours) blocks the write forever; the flasher must still get the port.
            tokio::select! {
                result = write_frame(port, &frame, paced.then_some(pace_rate(baud.rate))) => {
                    if let Err(error) = result {
                        pending.push_front(frame);
                        warn!(port = %port_name, %error, "link write failed, reconnecting");
                        return SessionEnd::Reconnect;
                    }
                }
                changed = suspend.changed() => {
                    if changed.is_err() {
                        return SessionEnd::Shutdown;
                    }
                    if *suspend.borrow() {
                        info!(port = %port_name, "serial port released for flashing");
                        return SessionEnd::Released;
                    }
                    pending.push_front(frame);
                }
            }
            continue;
        }

        let deadline = firmware.deadline();
        let rate_deadline = baud.deadline();
        tokio::select! {
            () = tokio::time::sleep_until(rate_deadline.unwrap_or_else(Instant::now)), if rate_deadline.is_some() => {
                warn!(port = %port_name, "the box didn't answer about the bridge's rate in time; staying at 115200");
                if let Err(error) = take_rate_steps(port, set_rate, baud.on_deadline(), paced, device_event_sender).await {
                    warn!(port = %port_name, %error, "link write failed, reconnecting");
                    return SessionEnd::Reconnect;
                }
            }
            frame = receiver.recv() => {
                match frame {
                    // Heartbeats and events for a box running other firmware are dropped, not held: they'd be stale.
                    Some(_) if firmware == Firmware::Foreign => {}
                    Some(frame) => pending.push_back(frame),
                    None => return SessionEnd::Shutdown,
                }
            }
            () = tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                if matches!(firmware, Firmware::Listening { .. }) {
                    firmware = Firmware::Asked { since: Instant::now() };
                    if !pending.iter().any(|frame| is_hello(frame)) {
                        pending.push_front(hello_frame());
                    }
                } else {
                    warn!(port = %port_name, "no firmware build after hello: the box runs other firmware, writing it nothing");
                    become_foreign(&mut firmware, pending, device_event_sender).await;
                }
            }
            changed = suspend.changed() => {
                if changed.is_err() {
                    return SessionEnd::Shutdown;
                }
                if *suspend.borrow() {
                    info!(port = %port_name, "serial port released for flashing");
                    return SessionEnd::Released;
                }
            }
            _ = lan_check.tick(), if probe_lan.is_some() => {
                if let Some(lan) = probe_lan
                    && lan::reachable(lan).await
                {
                    return SessionEnd::Lan;
                }
            }
            _ = cable_check.tick(), if watch_cable.is_some() => {
                if watch_cable.is_some_and(|config| matches!(find_port(config), Ok(Found::One(_)))) {
                    return SessionEnd::Usb;
                }
                if relay_switch.is_some_and(|enabled| !enabled.load(Ordering::Relaxed)) {
                    info!("the Relay was turned off, leaving it");
                    return SessionEnd::Reconnect;
                }
            }
            result = port.read(&mut read_buffer) => {
                match result {
                    Ok(0) => {
                        warn!(port = %port_name, "link closed, reconnecting");
                        return SessionEnd::Reconnect;
                    }
                    Ok(count) if set_rate.is_some() && baud.rate != BAUD_RATE && garbled(&read_buffer[..count]) => {
                        // Listened to again from the start: the box may have restarted.
                        warn!(port = %port_name, "the bridge reads garbled at the fast rate; back to 115200");
                        line_buffer.clear();
                        firmware = Firmware::Listening { since: Instant::now() };
                        if let Err(error) = take_rate_steps(port, set_rate, baud.on_garbled(), paced, device_event_sender).await {
                            warn!(port = %port_name, %error, "link write failed, reconnecting");
                            return SessionEnd::Reconnect;
                        }
                    }
                    Ok(count) => {
                        let before = firmware;
                        let rate_lines = process_device_bytes(&read_buffer[..count], &mut line_buffer, &mut firmware, device_event_sender);
                        let mut steps = Vec::new();
                        if set_rate.is_some() {
                            for line in rate_lines {
                                if line.contains(ROM_BANNER) {
                                    baud.on_restart();
                                } else {
                                    steps.extend(baud.on_line(&line, Instant::now()));
                                }
                            }
                            if firmware == Firmware::Ours && before != Firmware::Ours {
                                steps.extend(baud.on_ours(Instant::now()));
                            }
                        }
                        if let Err(error) = take_rate_steps(port, set_rate, steps, paced, device_event_sender).await {
                            warn!(port = %port_name, %error, "link write failed, reconnecting");
                            return SessionEnd::Reconnect;
                        }
                        match (before, firmware) {
                            (Firmware::Foreign, Firmware::Foreign) => {}
                            (_, Firmware::Foreign) => {
                                warn!(port = %port_name, "the box logs like other ESP-IDF firmware, writing it nothing");
                                become_foreign(&mut firmware, pending, device_event_sender).await;
                            }
                            (Firmware::Ours, Firmware::Listening { .. }) => {
                                info!(port = %port_name, "the box restarted, listening before writing to it");
                            }
                            (Firmware::Listening { .. } | Firmware::Asked { .. } | Firmware::Foreign, Firmware::Ours) => {
                                info!(port = %port_name, "the box runs VibeBuddy firmware");
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        warn!(port = %port_name, %error, "link read failed, reconnecting");
                        return SessionEnd::Reconnect;
                    }
                }
            }
        }
    }
}

async fn become_foreign(
    firmware: &mut Firmware,
    pending: &mut VecDeque<Vec<u8>>,
    device_event_sender: &mpsc::Sender<DeviceMessage>,
) {
    *firmware = Firmware::Foreign;
    pending.clear();
    let _ = device_event_sender.send(DeviceMessage::ForeignFirmware).await;
}

/// Bridge ports are written in line-rate chunks, `pace` being the rate the port runs at; native USB ports (None) get
/// the whole frame at once.
async fn write_frame<P: AsyncWrite + Unpin>(port: &mut P, frame: &[u8], pace: Option<u32>) -> std::io::Result<()> {
    let Some(baud) = pace else {
        // No flush: on a serial port it is tcdrain(), a blocking call that waits until the box has taken the bytes.
        // While the box dumps a screenshot it takes nothing, so a heartbeat's flush held this worker for two seconds;
        // nobody read the port meanwhile, the kernel stopped taking the box's output, and the box dropped the rest of
        // the frame (seen on Omarchy, 2026-10-01). The kernel queues the frame either way.
        return port.write_all(frame).await;
    };
    for piece in frame.chunks(PACE_PIECE_BYTES) {
        port.write_all(piece).await?;
        port.flush().await?;
        tokio::time::sleep(piece_delay(piece.len(), baud)).await;
    }
    Ok(())
}

/// How long this many bytes take at this baud rate, plus a little headroom. Flashing uses a separate
/// synchronous serial path with the same formula.
pub fn piece_delay(bytes: usize, baud: u32) -> Duration {
    Duration::from_micros(bytes as u64 * 10 * 1_000_000 / u64::from(baud)) + PACE_MARGIN
}

/// Over the bridge, write at most this many bytes, then wait for them to clear.
pub const PACE_PIECE_BYTES: usize = 128;

/// The rate whose wire time paces writes to the bridge running at `port_rate`. On macOS, that rate. On Linux,
/// always 115200: paced faster, at 921600 or even at 460800 wire time, writes through `cdc_acm` arrive damaged (on
/// Omarchy, 2026-10-29: a Character pack had 170 bad lines and took 5:41, against none and 2:09 at 115200).
fn pace_rate(port_rate: u32) -> u32 {
    if cfg!(target_os = "macos") { port_rate } else { BAUD_RATE }
}

/// Only the BOX's UART bridge needs chunking; Espressif's native USB port has its own flow control.
fn needs_pacing(vid: u16, pid: u16) -> bool {
    (vid, pid) == (QINHENG_VID, USB_SINGLE_SERIAL_PID)
}

#[derive(Clone, Debug, PartialEq)]
struct PortChoice {
    name: String,
    paced: bool,
    usb_serial: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Found {
    None,
    One(PortChoice),
    /// More than one could be the box, and the remembered one isn't among them.
    Several(Vec<PortChoice>),
}

fn find_port(config: &SerialConfig) -> Result<Found, String> {
    let ports = tokio_serial::available_ports().map_err(|error| error.to_string())?;
    if let Some(port) = &config.explicit_port {
        // A pinned port that isn't there is no box on the cable, so the network may be tried.
        if !std::path::Path::new(port).exists() {
            return Ok(Found::None);
        }
        // An explicitly given port still has its VID/PID looked up to choose pacing; if the system can't find it, assume the bridge, the most conservative case.
        let paced = ports
            .iter()
            .find(|candidate| &candidate.port_name == port)
            .and_then(|candidate| match &candidate.port_type {
                SerialPortType::UsbPort(info) => Some(needs_pacing(info.vid, info.pid)),
                _ => None,
            })
            .unwrap_or(true);
        return Ok(Found::One(PortChoice { name: port.clone(), paced, usb_serial: None }));
    }

    let mut matches: Vec<PortChoice> = ports
        .into_iter()
        .filter_map(|port| match port.port_type {
            SerialPortType::UsbPort(info)
                if is_supported_usb_port(info.vid, info.pid)
                    && config.usb_serial.as_ref().is_none_or(|expected| {
                        info.serial_number
                            .as_ref()
                            .is_some_and(|actual| serials_equal(actual, expected))
                    }) =>
            {
                Some(PortChoice { name: port.port_name, paced: needs_pacing(info.vid, info.pid), usb_serial: info.serial_number })
            }
            _ => None,
        })
        .collect();

    let callout_matches: Vec<PortChoice> = matches
        .iter()
        .filter(|choice| choice.name.starts_with("/dev/cu."))
        .cloned()
        .collect();
    if !callout_matches.is_empty() {
        matches = callout_matches;
    }
    matches.sort_by(|left, right| left.name.cmp(&right.name));
    matches.dedup_by(|left, right| left.name == right.name);
    Ok(choose_port(matches, config.known_box_serial().as_deref()))
}

/// One candidate is the box; several are, too, when one of them is the box we have seen before.
fn choose_port(mut matches: Vec<PortChoice>, known_box: Option<&str>) -> Found {
    if matches.len() > 1
        && let Some(known) = known_box
    {
        let ours: Vec<PortChoice> = matches
            .iter()
            .filter(|choice| choice.usb_serial.as_deref().is_some_and(|serial| serials_equal(serial, known)))
            .cloned()
            .collect();
        if ours.len() == 1 {
            matches = ours;
        }
    }
    match matches.len() {
        0 => Found::None,
        1 => Found::One(matches.remove(0)),
        _ => Found::Several(matches),
    }
}

fn is_supported_usb_port(vid: u16, pid: u16) -> bool {
    matches!(
        (vid, pid),
        (ESPRESSIF_VID, USB_SERIAL_JTAG_PID) | (QINHENG_VID, USB_SINGLE_SERIAL_PID)
    )
}

fn open_port(port_name: &str) -> tokio_serial::Result<SerialStream> {
    tokio_serial::new(port_name, BAUD_RATE).open_native_async()
}

pub fn serials_equal(actual: &str, expected: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    normalize(actual) == normalize(expected)
}

/// Hands the box's whole lines on, and returns the ones the session acts on itself: the ROM banner and the lines about
/// the bridge's rate.
fn process_device_bytes(
    bytes: &[u8],
    line_buffer: &mut Vec<u8>,
    firmware: &mut Firmware,
    event_sender: &mpsc::Sender<DeviceMessage>,
) -> Vec<String> {
    let mut own = Vec::new();
    for byte in bytes {
        if *byte == b'\n' {
            let length = if line_buffer.last() == Some(&b'\r') {
                line_buffer.len().saturating_sub(1)
            } else {
                line_buffer.len()
            };
            let line = &line_buffer[..length];
            let text = String::from_utf8_lossy(line);
            let rate = rate_line(&text);
            if text.contains(ROM_BANNER) {
                own.push(text.into_owned());
            } else if rate.starts_with(BAUD_PREFIX) || rate == BAUD_UNKNOWN {
                own.push(rate.to_owned());
            }
            process_device_line(line, firmware, event_sender);
            line_buffer.clear();
        } else if line_buffer.len() < 4096 {
            line_buffer.push(*byte);
        } else {
            line_buffer.clear();
            warn!("device output line exceeded 4096 bytes, dropped");
        }
    }
    own
}

fn process_device_line(line: &[u8], firmware: &mut Firmware, event_sender: &mpsc::Sender<DeviceMessage>) {
    if line.is_empty() {
        return;
    }
    *firmware = firmware.after_line(&String::from_utf8_lossy(line), Instant::now());
    // Other firmware's output (Muse's log) must not pass for our diagnostic lines or events.
    if *firmware == Firmware::Foreign {
        debug!(message = %String::from_utf8_lossy(line), "output from a box running other firmware");
        return;
    }
    let message = if line.first() == Some(&b'{') {
        match serde_json::from_slice::<Event>(line) {
            Ok(event) => match event.validate() {
                Ok(()) => DeviceMessage::Event(event),
                Err(error) => {
                    warn!(%error, "invalid device event");
                    return;
                }
            },
            Err(error) => {
                warn!(%error, "cannot parse device event JSON");
                return;
            }
        }
    } else {
        let line = String::from_utf8_lossy(line).into_owned();
        // Screenshot rows (hundreds of them) stay out of the log; their BEGIN and END lines and every other
        // diagnostic line are still logged, so a screenshot that never finishes shows where it stopped. So do the
        // lines of recorded audio, which carry what the user said.
        let screenshot_row = line.starts_with("SHOT ") && !line.starts_with("SHOT BEGIN") && line != "SHOT END";
        if !screenshot_row && !line.starts_with("ECHO ") && !line.starts_with("VOICE DATA ") {
            info!(message = %line, "device message");
        }
        DeviceMessage::Line(line)
    };
    match event_sender.try_send(message) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => warn!("device message queue full, dropped"),
        Err(mpsc::error::TrySendError::Closed(_)) => warn!("device message receiver closed"),
    }
}

/// Whether to leave the box alone for now: it answered busy less than [`BUSY_RETRY`] ago, and the user has not asked
/// to take it over. The cable is still looked at all the while.
fn leave_busy_box_alone(busy_retry: Option<Instant>, now: Instant, takeover: bool) -> bool {
    !takeover && busy_retry.is_some_and(|at| now < at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, serial: &str) -> PortChoice {
        PortChoice { name: name.to_owned(), paced: false, usb_serial: Some(serial.to_owned()) }
    }

    #[test]
    fn a_busy_box_is_left_alone_until_the_retry_or_a_takeover() {
        let now = Instant::now();
        assert!(!leave_busy_box_alone(None, now, false), "a box that never said busy is asked");
        let retry = Some(now + BUSY_RETRY);
        assert!(leave_busy_box_alone(retry, now + RECONNECT_DELAY, false), "not asked again half a second later");
        assert!(!leave_busy_box_alone(retry, now + RECONNECT_DELAY, true), "a takeover asks at once");
        assert!(!leave_busy_box_alone(retry, now + BUSY_RETRY, false), "asked again once the wait is over");
    }

    #[test]
    fn with_another_device_plugged_in_the_known_box_wins() {
        let both = vec![candidate("/dev/cu.usbmodem1101", "30:ED:A0:A4:0D:08"), candidate("/dev/cu.usbmodem8401", "98:88:E0:06:8B:CC")];
        let Found::One(chosen) = choose_port(both.clone(), Some("98:88:e0:06:8b:cc")) else { panic!("the known box wins") };
        assert_eq!(chosen.name, "/dev/cu.usbmodem8401");
        assert_eq!(choose_port(both.clone(), None), Found::Several(both.clone()), "no box known yet: don't guess, offer them");
        assert_eq!(choose_port(both.clone(), Some("11:22:33:44:55:66")), Found::Several(both), "the known box isn't one of them");
        let alone = vec![candidate("/dev/cu.usbmodem1101", "30:ED:A0:A4:0D:08")];
        let Found::One(lone) = choose_port(alone, Some("98:88:E0:06:8B:CC")) else { panic!("a lone device is still tried") };
        assert_eq!(lone.name, "/dev/cu.usbmodem1101");
    }

    #[test]
    fn usb_serial_comparison_ignores_case_and_separators() {
        assert!(serials_equal("98:88:E0:06:8B:CC", "9888e0068bcc"));
    }

    #[test]
    fn supports_both_native_usb_and_box_uart_bridge() {
        assert!(is_supported_usb_port(0x303a, 0x1001));
        assert!(is_supported_usb_port(0x1a86, 0x55d3));
        assert!(!is_supported_usb_port(0x1234, 0x5678));
    }

    #[test]
    fn chunked_button_event_reaches_the_mac_event_queue() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut buffer = Vec::new();

        process_device_bytes(
            br#"{"version":1,"event":"button","button":"K"#,
            &mut buffer,
            &mut Firmware::Ours,
            &sender,
        );
        assert!(receiver.try_recv().is_err(), "a partial line must not become an event early");
        process_device_bytes(b"2\",\"action\":\"press\"}\r\n", &mut buffer, &mut Firmware::Ours, &sender);

        let DeviceMessage::Event(event) = receiver.try_recv().expect("a complete line should enter the event queue")
        else {
            panic!("a JSON line should become an event");
        };
        assert_eq!(event.event, "button");
        assert_eq!(event.extra["button"], "K2");
        assert_eq!(event.extra["action"], "press");
    }

    #[test]
    fn only_the_uart_bridge_needs_pacing() {
        assert!(needs_pacing(0x1a86, 0x55d3));
        assert!(!needs_pacing(0x303a, 0x1001));
    }

    #[test]
    fn a_piece_waits_for_its_own_wire_time_plus_margin() {
        // 128 bytes × 10 bits / 115200 ≈ 11.1 ms, plus 1 ms headroom.
        let delay = piece_delay(128, BAUD_RATE);
        assert!(delay >= Duration::from_micros(12_100) && delay <= Duration::from_micros(12_200), "{delay:?}");
        // At 921600 about 1.4 ms, plus the same headroom.
        let delay = piece_delay(128, 921_600);
        assert!(delay >= Duration::from_micros(2_380) && delay <= Duration::from_micros(2_400), "{delay:?}");
    }

    #[test]
    fn only_macos_paces_at_the_fast_rate() {
        assert_eq!(pace_rate(BAUD_RATE), BAUD_RATE);
        let expected = if cfg!(target_os = "macos") { FAST_BAUD_RATE } else { BAUD_RATE };
        assert_eq!(pace_rate(FAST_BAUD_RATE), expected);
    }

    #[test]
    fn diagnostic_lines_arrive_as_lines_not_events() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut buffer = Vec::new();

        process_device_bytes(b"DISPLAY READY\n", &mut buffer, &mut Firmware::Ours, &sender);

        match receiver.try_recv().expect("diagnostic lines should reach the Mac too") {
            DeviceMessage::Line(line) => assert_eq!(line, "DISPLAY READY"),
            other => panic!("a diagnostic line should not be {other:?}"),
        }
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn a_peer_that_never_reads_cannot_keep_the_port_from_the_flasher() {
        // Firmware that isn't ours may never drain the USB serial, so a write can block forever: here hello, the
        // first thing written once the box has been listened to.
        tokio::time::pause();
        let (mut port, _peer_never_reads) = tokio::io::duplex(16);
        let mut pending = VecDeque::from([hello_frame()]);
        let (_frame_sender, mut receiver) = mpsc::channel(1);
        let (device_sender, _device_receiver) = mpsc::channel(1);
        let (suspend_sender, mut suspend) = watch::channel(false);

        let session = run_session(&mut port, "test", false, None, Firmware::Listening { since: Instant::now() }, None, None, None, &mut pending, &mut receiver, &device_sender, &mut suspend);
        let release = async {
            tokio::time::sleep(LISTEN_FIRST + Duration::from_millis(50)).await;
            suspend_sender.send(true).unwrap();
            std::future::pending::<()>().await;
        };
        let end = tokio::time::timeout(LISTEN_FIRST + Duration::from_secs(1), async {
            tokio::select! {
                end = session => end,
                _ = release => unreachable!(),
            }
        })
        .await
        .expect("a stuck write must not stop the worker from releasing the port");
        assert_eq!(end, SessionEnd::Released);
    }

    fn heartbeat_frame() -> Vec<u8> {
        b"{\"version\":1,\"event\":\"device.heartbeat\",\"build\":\"abc\",\"hour\":9,\"day\":20261007}\n".to_vec()
    }

    /// Everything the session has written to the box so far.
    async fn written(peer: &mut tokio::io::DuplexStream) -> String {
        let mut buffer = vec![0_u8; 8192];
        match tokio::time::timeout(Duration::from_millis(10), peer.read(&mut buffer)).await {
            Ok(Ok(count)) => String::from_utf8_lossy(&buffer[..count]).into_owned(),
            _ => String::new(),
        }
    }

    struct Session {
        peer: tokio::io::DuplexStream,
        frames: mpsc::Sender<Vec<u8>>,
        device: mpsc::Receiver<DeviceMessage>,
        _suspend: watch::Sender<bool>,
        task: tokio::task::JoinHandle<SessionEnd>,
    }

    /// A session as the daemon starts one: hello queued on connecting, a heartbeat behind it.
    async fn connected() -> Session {
        let (mut port, peer) = tokio::io::duplex(8192);
        let (frames, mut receiver) = mpsc::channel(16);
        let (device_sender, device) = mpsc::channel(64);
        let (suspend_sender, mut suspend) = watch::channel(false);
        frames.send(hello_frame()).await.unwrap();
        frames.send(heartbeat_frame()).await.unwrap();
        let task = tokio::spawn(async move {
            let mut pending = VecDeque::new();
            run_session(&mut port, "test", false, None, Firmware::Listening { since: Instant::now() }, None, None, None, &mut pending, &mut receiver, &device_sender, &mut suspend).await
        });
        Session { peer, frames, device, _suspend: suspend_sender, task }
    }

    fn drain(device: &mut mpsc::Receiver<DeviceMessage>) -> Vec<DeviceMessage> {
        std::iter::from_fn(|| device.try_recv().ok()).collect()
    }

    // What a box shared with Muse prints from reset until Muse's heartbeat, cut down.
    const MUSE_BOOT: &[u8] = b"ESP-ROM:esp32s3-20210327\r\nI (27) boot: ESP-IDF v6.0.1 2nd stage bootloader\r\n\
I (452) esp_image: segment 0: paddr=00020020\r\nI (571) esp_psram: Found 8MB PSRAM device\r\n\
I (911) muse: board: ALIENTEK ATK-DNESP32S3-BOX\r\n";

    #[test]
    fn only_an_esp_idf_application_log_gives_other_firmware_away() {
        assert!(is_esp_idf_app_log("I (911) muse: board: ALIENTEK ATK-DNESP32S3-BOX"));
        assert!(is_esp_idf_app_log("W (1450) wifi:Password length matches WPA2 standards"));
        // The bootloader both firmwares boot through.
        assert!(!is_esp_idf_app_log("I (27) boot: ESP-IDF v6.0.1 2nd stage bootloader"));
        assert!(!is_esp_idf_app_log("I (35) boot.esp32s3: Boot SPI Speed : 80MHz"));
        assert!(!is_esp_idf_app_log("I (452) esp_image: segment 0: paddr=00020020"));
        // Ours.
        assert!(!is_esp_idf_app_log("TALLY LOADED 0 0S DAY 20261007"));
        assert!(!is_esp_idf_app_log("DISPLAY READY BUILD v0.3.2 2026-10-07 09:00"));
        assert!(!is_esp_idf_app_log("I (x) muse: not a tick count"));
    }

    /// Our box booting the OTA layout (ADR-0013), captured on a box on 2026-10-12: the ESP-IDF v6.0.1
    /// bootloader with rollback logs its partition table and image segments before our first line.
    #[test]
    fn our_rollback_bootloader_does_not_pass_for_other_firmware() {
        let boot = include_str!("fixtures/box-boot-ota-layout.txt");
        let mut firmware = Firmware::Listening { since: Instant::now() };
        for line in boot.lines() {
            assert!(!is_esp_idf_app_log(line), "taken for another firmware's log: {line}");
            firmware = firmware.after_line(line, Instant::now());
            assert_ne!(firmware, Firmware::Foreign, "after {line}");
        }
        assert_eq!(firmware, Firmware::Ours);
    }

    #[tokio::test(start_paused = true)]
    async fn a_box_running_muse_is_written_nothing_not_even_hello() {
        let mut session = connected().await;
        session.peer.write_all(MUSE_BOOT).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        session.frames.send(heartbeat_frame()).await.unwrap();
        // Long past listening and asking: still nothing, and no probing.
        tokio::time::sleep(Duration::from_secs(60)).await;

        assert_eq!(written(&mut session.peer).await, "", "Muse's console reads letters as keys");
        let messages = drain(&mut session.device);
        assert!(messages.iter().any(|message| matches!(message, DeviceMessage::ForeignFirmware)), "{messages:?}");
        assert!(
            !messages.iter().any(|message| matches!(message, DeviceMessage::Line(line) if line.contains("muse"))),
            "Muse's log is not ours to read: {messages:?}"
        );
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn our_firmware_booting_gets_everything_without_waiting() {
        let mut session = connected().await;
        session
            .peer
            .write_all(b"ESP-ROM:esp32s3-20210327\r\nI (27) boot: ESP-IDF v6.1 2nd stage bootloader\r\nTALLY LOADED 0 0S DAY 20261007\r\nDISPLAY READY BUILD abc 2026-10-07 09:00\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let output = written(&mut session.peer).await;
        assert!(output.contains("device.hello") && output.contains("device.heartbeat"), "{output}");
        assert!(output.find("device.hello") < output.find("device.heartbeat"), "in order: {output}");
        let messages = drain(&mut session.device);
        assert!(messages.iter().any(|message| matches!(message, DeviceMessage::Line(line) if line.starts_with("TALLY"))));
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_box_is_listened_to_then_asked_once() {
        // Our firmware on the UART bridge: opening the port doesn't reset it, and idle it says nothing.
        let mut session = connected().await;
        tokio::time::sleep(LISTEN_FIRST - Duration::from_millis(100)).await;
        assert_eq!(written(&mut session.peer).await, "", "nothing before listening is over");

        tokio::time::sleep(Duration::from_millis(200)).await;
        let output = written(&mut session.peer).await;
        assert_eq!(output.matches("device.hello").count(), 1, "{output}");
        assert!(!output.contains("heartbeat"), "{output}");

        session.peer.write_all(b"DISPLAY READY BUILD abc 2026-10-07 09:00\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(written(&mut session.peer).await.contains("device.heartbeat"));
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(!drain(&mut session.device).iter().any(|message| matches!(message, DeviceMessage::ForeignFirmware)));
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_box_that_never_answers_hello_is_left_alone() {
        let mut session = connected().await;
        tokio::time::sleep(LISTEN_FIRST + FOREIGN_FIRMWARE_AFTER + Duration::from_secs(60)).await;
        let output = written(&mut session.peer).await;
        assert_eq!(output.matches("device.hello").count(), 1, "asked once, never probed again: {output}");
        assert!(!output.contains("heartbeat"), "{output}");
        assert!(drain(&mut session.device).iter().any(|message| matches!(message, DeviceMessage::ForeignFirmware)));
        session.task.abort();
    }

    fn frame_named(step: &RateStep) -> Option<String> {
        match step {
            RateStep::Write(frame) => serde_json::from_slice::<Event>(frame).ok().map(|event| event.event),
            _ => None,
        }
    }

    #[test]
    fn the_bridge_moves_to_the_fast_rate_once_the_box_confirms_it() {
        let now = Instant::now();
        let mut baud = BaudSwitch::new();
        let ask = baud.on_ours(now);
        assert_eq!(ask.len(), 1);
        let RateStep::Write(request) = &ask[0] else { panic!("{ask:?}") };
        let request: Event = serde_json::from_slice(request).unwrap();
        assert_eq!((request.event.as_str(), &request.extra["rate"]), ("device.baud", &serde_json::json!(921_600)));
        assert!(baud.holds_writes(), "nothing else goes out while the rate changes");
        assert!(baud.on_ours(now).is_empty(), "asked once");

        let moved = baud.on_line("BAUD SWITCH 921600\r", now);
        assert_eq!(moved[0], RateStep::SetRate(FAST_BAUD_RATE));
        assert_eq!(frame_named(&moved[1]).as_deref(), Some("device.baud.check"));
        assert!(baud.holds_writes());
        assert_eq!(baud.on_line("BAUD 921600", now), [RateStep::Settled { baud: FAST_BAUD_RATE, switches: true }]);
        assert!(!baud.holds_writes() && baud.deadline().is_none());
    }

    #[test]
    fn firmware_without_the_switch_stays_at_115200_at_once_or_after_the_wait() {
        let now = Instant::now();
        let mut baud = BaudSwitch::new();
        baud.on_ours(now);
        // As to any event it doesn't know.
        assert_eq!(baud.on_line("EVENT device.baud", now), [RateStep::Settled { baud: BAUD_RATE, switches: false }]);
        assert!(!baud.holds_writes());

        let mut baud = BaudSwitch::new();
        baud.on_ours(now);
        assert_eq!(baud.deadline(), Some(now + BAUD_ANSWER_WITHIN));
        assert_eq!(baud.on_deadline(), [RateStep::Settled { baud: BAUD_RATE, switches: false }]);
        assert_eq!(baud.rate, BAUD_RATE);
        assert!(baud.on_ours(now).is_empty(), "settled until the box restarts");
    }

    #[test]
    fn an_unconfirmed_switch_goes_back_to_115200_and_is_tried_again_after_a_restart() {
        let now = Instant::now();
        let mut baud = BaudSwitch::new();
        for _ in 0..BAUD_TRIES {
            baud.on_ours(now);
            baud.on_line("BAUD SWITCH 921600", now);
            assert_eq!(baud.on_deadline(), [RateStep::SetRate(BAUD_RATE), RateStep::Settled { baud: BAUD_RATE, switches: true }]);
            assert_eq!(baud.rate, BAUD_RATE);
        }
        assert!(baud.on_ours(now).is_empty(), "a bridge that can't hold the rate stays at 115200");
        baud.on_restart();
        assert_eq!(baud.on_ours(now).len(), 1, "asked again once the box restarted");
    }

    #[test]
    fn the_box_going_back_or_garbled_bytes_bring_the_port_back_to_115200() {
        let now = Instant::now();
        let mut baud = BaudSwitch::new();
        assert!(baud.on_garbled().is_empty(), "already at 115200");
        assert!(baud.on_line("BAUD 115200", now).is_empty());
        baud.on_ours(now);
        baud.on_line("BAUD SWITCH 921600", now);
        baud.on_line("BAUD 921600", now);
        assert_eq!(baud.on_line("BAUD 115200", now), [RateStep::SetRate(BAUD_RATE), RateStep::Settled { baud: BAUD_RATE, switches: true }]);
        baud.on_ours(now);
        baud.on_line("BAUD SWITCH 921600", now);
        baud.on_line("BAUD 921600", now);
        assert_eq!(baud.on_garbled(), [RateStep::SetRate(BAUD_RATE), RateStep::Settled { baud: BAUD_RATE, switches: true }]);
        assert_eq!(baud.on_ours(now).len(), 1, "and asks again when the box is found again");
    }

    #[test]
    fn only_bytes_our_firmware_never_sends_count_as_garbled() {
        assert!(!garbled(b"DISPLAY READY BUILD abc\r\n"));
        assert!(!garbled("WIFI SEEN 家 -40\n".as_bytes()), "UTF-8 is text");
        assert!(garbled(&[0x00, 0x80, 0x00]));
        assert!(garbled(&[0x41, 0xfe]));
    }

    #[test]
    fn our_firmware_back_at_115200_is_asked_at_once() {
        let now = Instant::now();
        let asked = Firmware::Foreign.after_line("BAUD 115200", now);
        assert!(asked.deadline().is_some_and(|deadline| deadline <= now), "{asked:?}");
        assert_eq!(Firmware::Ours.after_line("BAUD 115200", now), Firmware::Ours);
    }

    #[test]
    fn a_rate_line_after_bytes_read_at_the_other_rate_still_counts() {
        // Seen on a box: its last bytes at 921600, read at 115200, ran into its `BAUD 115200`.
        let (sender, _receiver) = mpsc::channel(16);
        let mut buffer = Vec::new();
        let mut firmware = Firmware::Asked { since: Instant::now() };
        let rate_lines = process_device_bytes(b"\xfe\xffBAUD 115200\r\n", &mut buffer, &mut firmware, &sender);
        assert_eq!(rate_lines, ["BAUD 115200"]);
        assert!(firmware.deadline().is_some_and(|deadline| deadline <= Instant::now()), "asked again at once: {firmware:?}");
        assert_eq!(rate_line("TITLE CC:UART BAUD 115200"), "TITLE CC:UART BAUD 115200", "only what is not text goes");
    }

    thread_local! {
        /// The rates a session on the bridge set its port to.
        static RATES: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record_rate(_port: &mut tokio::io::DuplexStream, baud: u32) -> std::io::Result<()> {
        RATES.with(|rates| rates.borrow_mut().push(baud));
        Ok(())
    }

    /// A session on the bridge, its rate recorded, with hello and a heartbeat queued.
    async fn on_bridge() -> Session {
        RATES.with(|rates| rates.borrow_mut().clear());
        let (mut port, peer) = tokio::io::duplex(8192);
        let (frames, mut receiver) = mpsc::channel(16);
        let (device_sender, device) = mpsc::channel(64);
        let (suspend_sender, mut suspend) = watch::channel(false);
        frames.send(hello_frame()).await.unwrap();
        frames.send(heartbeat_frame()).await.unwrap();
        let task = tokio::spawn(async move {
            let mut pending = VecDeque::new();
            let set_rate: RateSetter<tokio::io::DuplexStream> = record_rate;
            run_session(&mut port, "test", true, Some(set_rate), Firmware::Listening { since: Instant::now() }, None, None, None, &mut pending, &mut receiver, &device_sender, &mut suspend).await
        });
        Session { peer, frames, device, _suspend: suspend_sender, task }
    }

    fn rates() -> Vec<u32> {
        RATES.with(|rates| rates.borrow().clone())
    }

    fn bridge_rates(messages: &[DeviceMessage]) -> Vec<(u32, bool)> {
        messages
            .iter()
            .filter_map(|message| match message {
                DeviceMessage::BridgeRate { baud, switches } => Some((*baud, *switches)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn on_the_bridge_our_firmware_is_moved_to_the_fast_rate_before_anything_else() {
        let mut session = on_bridge().await;
        session.peer.write_all(b"ESP-ROM:esp32s3-20210327\r\nDISPLAY READY BUILD abc 2026-10-07 09:00\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let output = written(&mut session.peer).await;
        assert!(output.contains("device.baud") && !output.contains("device.hello"), "the request alone: {output}");

        session.peer.write_all(b"BAUD SWITCH 921600\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(rates(), [FAST_BAUD_RATE]);
        let output = written(&mut session.peer).await;
        assert!(output.contains("device.baud.check") && !output.contains("device.hello"), "{output}");

        session.peer.write_all(b"BAUD 921600\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let output = written(&mut session.peer).await;
        assert!(output.find("device.hello") < output.find("device.heartbeat"), "then the rest, in order: {output}");
        assert_eq!(bridge_rates(&drain(&mut session.device)), [(FAST_BAUD_RATE, true)]);

        // The box restarts and comes back at 115200: what the port reads at 921600 is noise.
        session.peer.write_all(&[0x00, 0xf8, 0x80, 0x00]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(rates(), [FAST_BAUD_RATE, BAUD_RATE]);
        assert_eq!(bridge_rates(&drain(&mut session.device)), [(BAUD_RATE, true)]);
        session.frames.send(heartbeat_frame()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(written(&mut session.peer).await, "", "listening again before writing");
        session.peer.write_all(b"DISPLAY READY BUILD abc 2026-10-07 09:00\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(written(&mut session.peer).await.contains("\"device.baud\""), "asked again");
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn on_the_bridge_firmware_without_the_switch_keeps_115200_and_gets_everything() {
        let mut session = on_bridge().await;
        session.peer.write_all(b"DISPLAY READY BUILD old 2026-09-01 09:00\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(written(&mut session.peer).await.contains("device.baud"));
        session.peer.write_all(b"EVENT device.baud\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let output = written(&mut session.peer).await;
        assert!(output.contains("device.hello") && output.contains("device.heartbeat"), "{output}");
        assert!(rates().is_empty(), "the port never moved");
        assert_eq!(bridge_rates(&drain(&mut session.device)), [(BAUD_RATE, false)]);
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn on_the_bridge_a_switch_left_unconfirmed_goes_back_to_115200() {
        let mut session = on_bridge().await;
        session.peer.write_all(b"DISPLAY READY BUILD abc 2026-10-07 09:00\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.peer.write_all(b"BAUD SWITCH 921600\r\n").await.unwrap();
        tokio::time::sleep(BAUD_ANSWER_WITHIN + Duration::from_millis(100)).await;
        assert_eq!(rates(), [FAST_BAUD_RATE, BAUD_RATE]);
        assert_eq!(bridge_rates(&drain(&mut session.device)), [(BAUD_RATE, true)]);
        assert!(written(&mut session.peer).await.contains("device.heartbeat"), "held frames go out at 115200");
        session.task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn a_reset_puts_the_box_in_doubt_until_it_says_what_it_runs() {
        // K1 + K2 on a box shared with Muse: our firmware resets into Muse while the port stays open.
        let mut session = connected().await;
        session.peer.write_all(b"DISPLAY READY BUILD abc 2026-10-07 09:00\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        written(&mut session.peer).await;
        drain(&mut session.device);

        session.peer.write_all(MUSE_BOOT).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.frames.send(heartbeat_frame()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(written(&mut session.peer).await, "");
        assert!(drain(&mut session.device).iter().any(|message| matches!(message, DeviceMessage::ForeignFirmware)));

        // And back: our firmware reports its build after its own reset.
        session.peer.write_all(b"ESP-ROM:esp32s3-20210327\r\nDISPLAY READY BUILD abc 2026-10-07 09:00\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.frames.send(heartbeat_frame()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(written(&mut session.peer).await.contains("device.heartbeat"));
        session.task.abort();
    }
}