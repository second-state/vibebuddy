use std::collections::VecDeque;
use std::env;
use std::time::Duration;

use beacon_protocol::Event;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio_serial::{SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::{info, warn};

const ESPRESSIF_VID: u16 = 0x303a;
const USB_SERIAL_JTAG_PID: u16 = 0x1001;
const QINHENG_VID: u16 = 0x1a86;
const USB_SINGLE_SERIAL_PID: u16 = 0x55d3;
const QUEUE_CAPACITY: usize = 64;
/// 截图一次 240 行、语音包写入每块一条回执，队列要装得下一屏。
const DEVICE_EVENT_CAPACITY: usize = 512;
const BAUD_RATE: u32 = 115_200;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
const CONNECT_SETTLE_DELAY: Duration = Duration::from_millis(1_500);
/// BOX 的 CH343 UART 桥吞不下一整行：主机一次推超过一两百字节，桥的缓冲
/// 就被冲掉——每 384 字节丢 32 字节再把后面 32 字节重复一遍，长度不变、
/// 内容错位。设备侧用直接轮询 FIFO 的对照实验证明无辜。桥接时按线速分段，
/// 每段写完等它在线上走完再写下一段。
const PACE_PIECE_BYTES: usize = 128;
const PACE_MARGIN: Duration = Duration::from_millis(1);

/// 设备到 Mac 的一切：JSON 事件、诊断行，以及链路本身的连与断。
#[derive(Clone, Debug)]
pub enum DeviceMessage {
    Event(Event),
    Line(String),
    /// `bridge` 为真表示接在 BOX 的 CH343 UART 桥上：写要分段，烧录要小块。
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
            explicit_port: env::var("BEACON_SERIAL_PORT").ok(),
            usb_serial: env::var("BEACON_USB_SERIAL").ok(),
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

    /// 让出串口给烧录：worker 关掉端口并停止重连，直到再次放开。
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
            // 烧录期间不碰串口；也不攒队列里的旧帧，设备重启后它们已经过时。
            pending.clear();
            if suspend.changed().await.is_err() {
                return;
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
                warn!(%error, "串口发现失败，稍后重试");
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };

        let mut port = match open_port(&port_name) {
            Ok(port) => port,
            Err(error) => {
                warn!(port = %port_name, %error, "打开串口失败，稍后重试");
                tokio::time::sleep(RECONNECT_DELAY).await;
                continue;
            }
        };
        info!(port = %port_name, paced, "串口已连接");
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
                    warn!(port = %port_name, %error, "串口写入失败，开始重连");
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
                        info!(port = %port_name, "串口让出给烧录");
                        break;
                    }
                }
                result = port.read(&mut read_buffer) => {
                    match result {
                        Ok(0) => {
                            warn!(port = %port_name, "串口已关闭，开始重连");
                            break;
                        }
                        Ok(count) => process_device_bytes(
                            &read_buffer[..count],
                            &mut line_buffer,
                            &device_event_sender,
                        ),
                        Err(error) => {
                            warn!(port = %port_name, %error, "串口读取失败，开始重连");
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

/// 桥接口按线速分段写；原生 USB 口整帧写。
async fn write_frame(port: &mut SerialStream, frame: &[u8], paced: bool) -> std::io::Result<()> {
    if !paced {
        port.write_all(frame).await?;
        return port.flush().await;
    }
    for piece in frame.chunks(PACE_PIECE_BYTES) {
        port.write_all(piece).await?;
        port.flush().await?;
        tokio::time::sleep(piece_delay(piece.len())).await;
    }
    Ok(())
}

/// 这么多字节在 115200 波特下需要多久走完，外加一点余量。
fn piece_delay(bytes: usize) -> Duration {
    Duration::from_micros(bytes as u64 * 10 * 1_000_000 / u64::from(BAUD_RATE)) + PACE_MARGIN
}

/// 只有 BOX 的 UART 桥需要分段；乐鑫原生 USB 口自己有流控。
fn needs_pacing(vid: u16, pid: u16) -> bool {
    (vid, pid) == (QINHENG_VID, USB_SINGLE_SERIAL_PID)
}

struct PortChoice {
    name: String,
    paced: bool,
}

fn find_port(config: &SerialConfig) -> Result<Option<PortChoice>, String> {
    if let Some(port) = &config.explicit_port {
        // 指定端口时不知道它背后是什么，按最保守的桥接节奏发。
        return Ok(Some(PortChoice { name: port.clone(), paced: true }));
    }

    let ports = tokio_serial::available_ports().map_err(|error| error.to_string())?;
    let mut matches: Vec<(String, bool)> = ports
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
                Some((port.port_name, needs_pacing(info.vid, info.pid)))
            }
            _ => None,
        })
        .collect();

    let callout_matches: Vec<(String, bool)> = matches
        .iter()
        .filter(|(port, _)| port.starts_with("/dev/cu."))
        .cloned()
        .collect();
    if !callout_matches.is_empty() {
        matches = callout_matches;
    }
    matches.sort();
    matches.dedup();

    match matches.as_slice() {
        [] => Ok(None),
        [(port, paced)] => Ok(Some(PortChoice { name: port.clone(), paced: *paced })),
        ports => Err(format!(
            "找到多个匹配设备：{}",
            ports.iter().map(|(port, _)| port.as_str()).collect::<Vec<_>>().join(", ")
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
            warn!("设备输出单行超过 4096 bytes，已丢弃");
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
                    warn!(%error, "设备事件无效");
                    return;
                }
            },
            Err(error) => {
                warn!(%error, "设备事件 JSON 无法解析");
                return;
            }
        }
    } else {
        let line = String::from_utf8_lossy(line).into_owned();
        // 截图的几百行不进日志，其余诊断行照旧记一笔。
        if !line.starts_with("SHOT ") && !line.starts_with("ECHO ") {
            info!(message = %line, "设备消息");
        }
        DeviceMessage::Line(line)
    };
    match event_sender.try_send(message) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => warn!("设备消息队列已满，已丢弃"),
        Err(mpsc::error::TrySendError::Closed(_)) => warn!("设备消息接收器已关闭"),
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
        assert!(receiver.try_recv().is_err(), "半行不能提前成为事件");
        process_device_bytes(b"2\",\"action\":\"press\"}\r\n", &mut buffer, &sender);

        let DeviceMessage::Event(event) = receiver.try_recv().expect("完整行应进入事件队列")
        else {
            panic!("JSON 行应成为事件");
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
        // 128 字节 × 10 位 / 115200 ≈ 11.1 ms，加 1 ms 余量。
        let delay = piece_delay(128);
        assert!(delay >= Duration::from_micros(12_100) && delay <= Duration::from_micros(12_200), "{delay:?}");
    }

    #[test]
    fn diagnostic_lines_arrive_as_lines_not_events() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut buffer = Vec::new();

        process_device_bytes(b"DISPLAY READY\n", &mut buffer, &sender);

        match receiver.try_recv().expect("诊断行也要送到 Mac 端") {
            DeviceMessage::Line(line) => assert_eq!(line, "DISPLAY READY"),
            other => panic!("诊断行不该是 {other:?}"),
        }
        assert!(buffer.is_empty());
    }
}
