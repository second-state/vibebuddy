use std::collections::VecDeque;
use std::env;
use std::time::Duration;

use beacon_protocol::Event;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_serial::{SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::{info, warn};

const ESPRESSIF_VID: u16 = 0x303a;
const USB_SERIAL_JTAG_PID: u16 = 0x1001;
const QUEUE_CAPACITY: usize = 64;
const DEVICE_EVENT_CAPACITY: usize = 16;
const BAUD_RATE: u32 = 115_200;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
const CONNECT_SETTLE_DELAY: Duration = Duration::from_millis(1_500);

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
}

impl SerialTransport {
    pub fn spawn(config: SerialConfig) -> (Self, mpsc::Receiver<Event>) {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (device_event_sender, device_event_receiver) = mpsc::channel(DEVICE_EVENT_CAPACITY);
        tokio::spawn(serial_worker(config, receiver, device_event_sender));
        (Self { sender }, device_event_receiver)
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
    device_event_sender: mpsc::Sender<Event>,
) {
    let mut pending: VecDeque<Vec<u8>> = VecDeque::new();

    loop {
        let port_name = match find_port(&config) {
            Ok(Some(port_name)) => port_name,
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
        info!(port = %port_name, "串口已连接");
        tokio::time::sleep(CONNECT_SETTLE_DELAY).await;

        let mut read_buffer = [0_u8; 256];
        let mut line_buffer = Vec::new();

        loop {
            if let Some(frame) = pending.pop_front() {
                if let Err(error) = port.write_all(&frame).await {
                    pending.push_front(frame);
                    warn!(port = %port_name, %error, "串口写入失败，开始重连");
                    break;
                }
                if let Err(error) = port.flush().await {
                    pending.push_front(frame);
                    warn!(port = %port_name, %error, "串口 flush 失败，开始重连");
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

        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

fn find_port(config: &SerialConfig) -> Result<Option<String>, String> {
    if let Some(port) = &config.explicit_port {
        return Ok(Some(port.clone()));
    }

    let ports = tokio_serial::available_ports().map_err(|error| error.to_string())?;
    let mut matches: Vec<String> = ports
        .into_iter()
        .filter_map(|port| match port.port_type {
            SerialPortType::UsbPort(info)
                if info.vid == ESPRESSIF_VID
                    && info.pid == USB_SERIAL_JTAG_PID
                    && config.usb_serial.as_ref().is_none_or(|expected| {
                        info.serial_number
                            .as_ref()
                            .is_some_and(|actual| serials_equal(actual, expected))
                    }) =>
            {
                Some(port.port_name)
            }
            _ => None,
        })
        .collect();

    let callout_matches: Vec<String> = matches
        .iter()
        .filter(|port| port.starts_with("/dev/cu."))
        .cloned()
        .collect();
    if !callout_matches.is_empty() {
        matches = callout_matches;
    }
    matches.sort();
    matches.dedup();

    match matches.as_slice() {
        [] => Ok(None),
        [port] => Ok(Some(port.clone())),
        ports => Err(format!("找到多个匹配设备：{}", ports.join(", "))),
    }
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
    event_sender: &mpsc::Sender<Event>,
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

fn process_device_line(line: &[u8], event_sender: &mpsc::Sender<Event>) {
    if line.is_empty() {
        return;
    }
    if line.first() == Some(&b'{') {
        match serde_json::from_slice::<Event>(line) {
            Ok(event) => match event.validate() {
                Ok(()) => match event_sender.try_send(event) {
                    Ok(()) => return,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!("设备事件队列已满，已丢弃事件");
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        warn!("设备事件接收器已关闭");
                        return;
                    }
                },
                Err(error) => warn!(%error, "设备事件无效"),
            },
            Err(error) => warn!(%error, "设备事件 JSON 无法解析"),
        }
        return;
    }
    let line = String::from_utf8_lossy(line);
    info!(message = %line, "设备消息");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_serial_comparison_ignores_case_and_separators() {
        assert!(serials_equal("98:88:E0:06:8B:CC", "9888e0068bcc"));
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

        let event = receiver.try_recv().expect("完整行应进入事件队列");
        assert_eq!(event.event, "button");
        assert_eq!(event.extra["button"], "K2");
        assert_eq!(event.extra["action"], "press");
    }

    #[test]
    fn diagnostic_lines_do_not_become_events() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut buffer = Vec::new();

        process_device_bytes(b"DISPLAY READY\n", &mut buffer, &sender);

        assert!(receiver.try_recv().is_err());
        assert!(buffer.is_empty());
    }
}
