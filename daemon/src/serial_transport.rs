use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use vibebuddy_protocol::Event;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_serial::{ClearBuffer, SerialPort, SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::{debug, info, warn};

const ESPRESSIF_VID: u16 = 0x303a;
const USB_SERIAL_JTAG_PID: u16 = 0x1001;
const QINHENG_VID: u16 = 0x1a86;
const USB_SINGLE_SERIAL_PID: u16 = 0x55d3;
const QUEUE_CAPACITY: usize = 64;
/// A screenshot is 240 lines and a voice-pack write gets one receipt per block; the queue must hold a full screen.
const DEVICE_EVENT_CAPACITY: usize = 512;
const BAUD_RATE: u32 = 115_200;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
const CONNECT_SETTLE_DELAY: Duration = Duration::from_millis(1_500);
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
    /// The box runs other firmware (see [`Firmware::Foreign`]): it's there, but gets nothing written.
    ForeignFirmware,
    Disconnected,
}

pub trait Transport: Send + Sync {
    fn send(&self, frame: Vec<u8>) -> Result<(), TransportError>;
}

#[derive(Clone, Debug, Default)]
pub struct SerialConfig {
    explicit_port: Option<String>,
    usb_serial: Option<String>,
    /// Where the USB serial number of the last device that proved to be the box is kept; see [`remember_box`].
    known_box: Option<PathBuf>,
}

impl SerialConfig {
    pub fn from_env(known_box: Option<PathBuf>) -> Self {
        Self {
            explicit_port: env::var("VIBEBUDDY_SERIAL_PORT").ok(),
            usb_serial: env::var("VIBEBUDDY_USB_SERIAL").ok(),
            known_box,
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
}

impl SerialTransport {
    pub fn spawn(config: SerialConfig) -> (Self, mpsc::Receiver<DeviceMessage>) {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (device_sender, device_receiver) = mpsc::channel(DEVICE_EVENT_CAPACITY);
        let (suspend, suspend_receiver) = watch::channel(false);
        tokio::spawn(serial_worker(config, receiver, device_sender, suspend_receiver));
        (Self { sender, suspend }, device_receiver)
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
        let PortChoice { name: port_name, paced, usb_serial } = match find_port(&config) {
            Ok(Some(choice)) => choice,
            Ok(None) => {
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
            Err(error) => {
                warn!(%error, "serial port discovery failed, retrying later");
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
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

        match run_session(&mut port, &port_name, paced, &mut pending, &mut receiver, &device_event_sender, &mut suspend)
            .await
        {
            SessionEnd::Shutdown => return,
            SessionEnd::Reconnect => {}
            // Closing a tty waits for unsent output to drain, which never happens if the peer doesn't read.
            SessionEnd::Released => drop(port.clear(ClearBuffer::Output)),
        }

        drop(port);
        let _ = device_event_sender.send(DeviceMessage::Disconnected).await;
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

fn hello_frame() -> Vec<u8> {
    Event::named("device.hello").to_ndjson().expect("hello always encodes")
}

fn is_hello(frame: &[u8]) -> bool {
    serde_json::from_slice::<Event>(frame).is_ok_and(|event| event.event == "device.hello")
}

/// Shuttles frames and device output over one open port until it fails or is handed to the flasher.
async fn run_session<P: AsyncRead + AsyncWrite + Unpin>(
    port: &mut P,
    port_name: &str,
    paced: bool,
    pending: &mut VecDeque<Vec<u8>>,
    receiver: &mut mpsc::Receiver<Vec<u8>>,
    device_event_sender: &mpsc::Sender<DeviceMessage>,
    suspend: &mut watch::Receiver<bool>,
) -> SessionEnd {
    let mut read_buffer = [0_u8; 256];
    let mut line_buffer = Vec::new();
    let mut firmware = Firmware::Listening { since: Instant::now() };

    loop {
        // Frames wait, in order, until the box is known to run our firmware; only hello may go first.
        let next = pending.iter().position(|frame| firmware.writes(frame)).and_then(|index| pending.remove(index));
        if let Some(frame) = next {
            // A peer that never reads (e.g. firmware that isn't ours) blocks the write forever; the flasher must still get the port.
            tokio::select! {
                result = write_frame(port, &frame, paced) => {
                    if let Err(error) = result {
                        pending.push_front(frame);
                        warn!(port = %port_name, %error, "serial write failed, reconnecting");
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
        tokio::select! {
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
            result = port.read(&mut read_buffer) => {
                match result {
                    Ok(0) => {
                        warn!(port = %port_name, "serial port closed, reconnecting");
                        return SessionEnd::Reconnect;
                    }
                    Ok(count) => {
                        let before = firmware;
                        process_device_bytes(&read_buffer[..count], &mut line_buffer, &mut firmware, device_event_sender);
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
                                info!(port = %port_name, "the box runs Vibe Buddy firmware");
                            }
                            _ => {}
                        }
                    }
                    Err(error) => {
                        warn!(port = %port_name, %error, "serial read failed, reconnecting");
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

/// Bridge ports are written in line-rate chunks; native USB ports get the whole frame at once.
async fn write_frame<P: AsyncWrite + Unpin>(port: &mut P, frame: &[u8], paced: bool) -> std::io::Result<()> {
    if !paced {
        // No flush: on a serial port it is tcdrain(), a blocking call that waits until the box has taken the bytes.
        // While the box dumps a screenshot it takes nothing, so a heartbeat's flush held this worker for two seconds;
        // nobody read the port meanwhile, the kernel stopped taking the box's output, and the box dropped the rest of
        // the frame (seen on Omarchy, 2026-10-01). The kernel queues the frame either way.
        return port.write_all(frame).await;
    }
    for piece in frame.chunks(PACE_PIECE_BYTES) {
        port.write_all(piece).await?;
        port.flush().await?;
        tokio::time::sleep(piece_delay(piece.len())).await;
    }
    Ok(())
}

/// How long this many bytes take at 115200 baud, plus a little headroom. Flashing uses a separate
/// synchronous serial path with the same formula.
pub fn piece_delay(bytes: usize) -> Duration {
    Duration::from_micros(bytes as u64 * 10 * 1_000_000 / u64::from(BAUD_RATE)) + PACE_MARGIN
}

/// Over the bridge, write at most this many bytes, then wait for them to clear.
pub const PACE_PIECE_BYTES: usize = 128;

/// Only the BOX's UART bridge needs chunking; Espressif's native USB port has its own flow control.
fn needs_pacing(vid: u16, pid: u16) -> bool {
    (vid, pid) == (QINHENG_VID, USB_SINGLE_SERIAL_PID)
}

#[derive(Clone)]
struct PortChoice {
    name: String,
    paced: bool,
    usb_serial: Option<String>,
}

fn find_port(config: &SerialConfig) -> Result<Option<PortChoice>, String> {
    let ports = tokio_serial::available_ports().map_err(|error| error.to_string())?;
    if let Some(port) = &config.explicit_port {
        // An explicitly given port still has its VID/PID looked up to choose pacing; if the system can't find it, assume the bridge, the most conservative case.
        let paced = ports
            .iter()
            .find(|candidate| &candidate.port_name == port)
            .and_then(|candidate| match &candidate.port_type {
                SerialPortType::UsbPort(info) => Some(needs_pacing(info.vid, info.pid)),
                _ => None,
            })
            .unwrap_or(true);
        return Ok(Some(PortChoice { name: port.clone(), paced, usb_serial: None }));
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
    choose_port(matches, config.known_box_serial().as_deref())
}

/// One candidate is the box; several are, too, when one of them is the box we have seen before.
fn choose_port(mut matches: Vec<PortChoice>, known_box: Option<&str>) -> Result<Option<PortChoice>, String> {
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
    match matches.as_slice() {
        [] => Ok(None),
        [choice] => Ok(Some(choice.clone())),
        choices => Err(format!(
            "found several matching devices: {}",
            choices.iter().map(|choice| choice.name.as_str()).collect::<Vec<_>>().join(", ")
        )),
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

fn serials_equal(actual: &str, expected: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    normalize(actual) == normalize(expected)
}

fn process_device_bytes(
    bytes: &[u8],
    line_buffer: &mut Vec<u8>,
    firmware: &mut Firmware,
    event_sender: &mpsc::Sender<DeviceMessage>,
) {
    for byte in bytes {
        if *byte == b'\n' {
            let length = if line_buffer.last() == Some(&b'\r') {
                line_buffer.len().saturating_sub(1)
            } else {
                line_buffer.len()
            };
            process_device_line(&line_buffer[..length], firmware, event_sender);
            line_buffer.clear();
        } else if line_buffer.len() < 4096 {
            line_buffer.push(*byte);
        } else {
            line_buffer.clear();
            warn!("device output line exceeded 4096 bytes, dropped");
        }
    }
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
        // diagnostic line are still logged, so a screenshot that never finishes shows where it stopped.
        let screenshot_row = line.starts_with("SHOT ") && !line.starts_with("SHOT BEGIN") && line != "SHOT END";
        if !screenshot_row && !line.starts_with("ECHO ") {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, serial: &str) -> PortChoice {
        PortChoice { name: name.to_owned(), paced: false, usb_serial: Some(serial.to_owned()) }
    }

    #[test]
    fn with_another_device_plugged_in_the_known_box_wins() {
        let both = vec![candidate("/dev/cu.usbmodem1101", "30:ED:A0:A4:0D:08"), candidate("/dev/cu.usbmodem8401", "98:88:E0:06:8B:CC")];
        let chosen = choose_port(both.clone(), Some("98:88:e0:06:8b:cc")).unwrap().unwrap();
        assert_eq!(chosen.name, "/dev/cu.usbmodem8401");
        assert!(choose_port(both.clone(), None).is_err(), "no box known yet: don't guess");
        assert!(choose_port(both, Some("11:22:33:44:55:66")).is_err(), "the known box isn't one of them");
        let alone = vec![candidate("/dev/cu.usbmodem1101", "30:ED:A0:A4:0D:08")];
        assert_eq!(choose_port(alone, Some("98:88:E0:06:8B:CC")).unwrap().unwrap().name, "/dev/cu.usbmodem1101", "a lone device is still tried");
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
        let delay = piece_delay(128);
        assert!(delay >= Duration::from_micros(12_100) && delay <= Duration::from_micros(12_200), "{delay:?}");
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

        let session = run_session(&mut port, "test", false, &mut pending, &mut receiver, &device_sender, &mut suspend);
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
            run_session(&mut port, "test", false, &mut pending, &mut receiver, &device_sender, &mut suspend).await
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