use std::collections::VecDeque;
use std::env;
use std::time::Duration;

use vibebuddy_protocol::Event;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio_serial::{SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::{info, warn};

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

/// Everything from the device to the Mac: JSON events, diagnostic lines, and the link connecting and dropping.
#[derive(Clone, Debug)]
pub enum DeviceMessage {
    Event(Event),
    Line(String),
    /// `bridge` means we're on the BOX's CH343 UART bridge: writes must be chunked and flashing uses small blocks.
    Connected { port: String, bridge: bool },
    Disconnected,
}

pub trait Transport: Send + Sync {
    fn send(&self, frame: Vec<u8>) -> Result<(), TransportError>;
}

#[derive(Clone, Debug, Default)]
pub struct SerialConfig {
    explicit_port: Option<String>,
    usb_serial: Option<String>,
}

impl SerialConfig {
    pub fn from_env() -> Self {
        Self {
            explicit_port: env::var("VIBEBUDDY_SERIAL_PORT").ok(),
            usb_serial: env::var("VIBEBUDDY_USB_SERIAL").ok(),
        }
    }
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
        let PortChoice { name: port_name, paced } = match find_port(&config) {
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
            .send(DeviceMessage::Connected { port: port_name.clone(), bridge: paced })
            .await;

        let mut read_buffer = [0_u8; 256];
        let mut line_buffer = Vec::new();

        loop {
            if let Some(frame) = pending.pop_front() {
                if let Err(error) = write_frame(&mut port, &frame, paced).await {
                    pending.push_front(frame);
                    warn!(port = %port_name, %error, "serial write failed, reconnecting");
                    break;
                }
                continue;
            }

            tokio::select! {
                frame = receiver.recv() => {
                    match frame {
                        Some(frame) => pending.push_back(frame),
                        None => return,
                    }
                }
                changed = suspend.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if *suspend.borrow() {
                        info!(port = %port_name, "serial port released for flashing");
                        break;
                    }
                }
                result = port.read(&mut read_buffer) => {
                    match result {
                        Ok(0) => {
                            warn!(port = %port_name, "serial port closed, reconnecting");
                            break;
                        }
                        Ok(count) => process_device_bytes(
                            &read_buffer[..count],
                            &mut line_buffer,
                            &device_event_sender,
                        ),
                        Err(error) => {
                            warn!(port = %port_name, %error, "serial read failed, reconnecting");
                            break;
                        }
                    }
                }
            }
        }

        drop(port);
        let _ = device_event_sender.send(DeviceMessage::Disconnected).await;
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// Bridge ports are written in line-rate chunks; native USB ports get the whole frame at once.
async fn write_frame(port: &mut SerialStream, frame: &[u8], paced: bool) -> std::io::Result<()> {
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
        return Ok(Some(PortChoice { name: port.clone(), paced }));
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
                Some(PortChoice { name: port.port_name, paced: needs_pacing(info.vid, info.pid) })
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
    event_sender: &mpsc::Sender<DeviceMessage>,
) {
    for byte in bytes {
        if *byte == b'\n' {
            let length = if line_buffer.last() == Some(&b'\r') {
                line_buffer.len().saturating_sub(1)
            } else {
                line_buffer.len()
            };
            process_device_line(&line_buffer[..length], event_sender);
            line_buffer.clear();
        } else if line_buffer.len() < 4096 {
            line_buffer.push(*byte);
        } else {
            line_buffer.clear();
            warn!("device output line exceeded 4096 bytes, dropped");
        }
    }
}

fn process_device_line(line: &[u8], event_sender: &mpsc::Sender<DeviceMessage>) {
    if line.is_empty() {
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
            &sender,
        );
        assert!(receiver.try_recv().is_err(), "a partial line must not become an event early");
        process_device_bytes(b"2\",\"action\":\"press\"}\r\n", &mut buffer, &sender);

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

        process_device_bytes(b"DISPLAY READY\n", &mut buffer, &sender);

        match receiver.try_recv().expect("diagnostic lines should reach the Mac too") {
            DeviceMessage::Line(line) => assert_eq!(line, "DISPLAY READY"),
            other => panic!("a diagnostic line should not be {other:?}"),
        }
        assert!(buffer.is_empty());
    }
}
