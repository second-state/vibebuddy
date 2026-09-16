//! 把语音包经串口协议写进设备：停等流控，每块带 CRC，设备每回一条才发下一块。
//! 协议见 docs/protocol.md「设备维护」。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use vibebuddy_protocol::Event;
use tokio::sync::broadcast;

use crate::serial_transport::{DeviceMessage, Transport, TransportError};

/// 每块原始字节数：base64 后加上 JSON 外壳仍在 1024 字节的一行上限内。
pub const CHUNK_BYTES: usize = 672;
const HEADER_BYTES: usize = 256;
/// 等设备回执的上限。擦除 2 MB 分区要两三秒，最后回读校验也要一会儿。
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// 从包头读音色 id；不是语音包就 None。
pub fn voice_id_of(pack: &[u8]) -> Option<String> {
    if pack.len() <= HEADER_BYTES || &pack[0..4] != b"VBVP" {
        return None;
    }
    let raw = &pack[16..48];
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    let id = std::str::from_utf8(&raw[..end]).ok()?;
    if id.is_empty() { None } else { Some(id.to_owned()) }
}

fn event(name: &str, fields: Vec<(&str, serde_json::Value)>) -> Event {
    let mut event = Event::named(name);
    event.extra = fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    event
}

pub fn begin_event(size: usize) -> Event {
    event("voice.begin", vec![("size", serde_json::json!(size))])
}

pub fn chunk_event(seq: usize, piece: &[u8]) -> Event {
    event(
        "voice.chunk",
        vec![
            ("seq", serde_json::json!(seq)),
            ("crc", serde_json::json!(crc32fast::hash(piece))),
            (
                "data",
                serde_json::json!(base64::engine::general_purpose::STANDARD.encode(piece)),
            ),
        ],
    )
}

pub fn end_event() -> Event {
    event("voice.end", vec![])
}

async fn send(transport: &Arc<dyn Transport>, event: Event) -> Result<(), String> {
    let frame = event.to_ndjson().map_err(|error| error.to_string())?;
    // 队列满只是设备还没消化完，等一等再塞。
    for _ in 0..200 {
        match transport.send(frame.clone()) {
            Ok(()) => return Ok(()),
            Err(TransportError::QueueFull) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(TransportError::Closed) => return Err("串口 worker 已停止".to_owned()),
        }
    }
    Err("设备发送队列一直满着".to_owned())
}

/// 等一条指定的设备回执。`voice.error` 与链路断开都算失败。
async fn wait_for(
    bus: &mut broadcast::Receiver<DeviceMessage>,
    wanted: &str,
    seq: Option<i64>,
) -> Result<Event, String> {
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        let message = match tokio::time::timeout_at(deadline, bus.recv()).await {
            Ok(Ok(message)) => message,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err("设备消息通道已关闭".to_owned()),
            Err(_) => return Err(format!("等待 {wanted} 超时")),
        };
        match message {
            DeviceMessage::Disconnected => return Err("链路断开".to_owned()),
            DeviceMessage::Event(event) if event.event == "voice.error" => {
                // `message` 是协议信封里的正式字段，不在 extra 里。
                let detail = event.message.as_deref().unwrap_or("未知错误");
                return Err(format!("设备拒绝：{detail}"));
            }
            DeviceMessage::Event(event) if event.event == wanted => {
                let matches = seq.is_none_or(|seq| {
                    event.extra.get("seq").and_then(|value| value.as_i64()) == Some(seq)
                });
                if matches {
                    return Ok(event);
                }
            }
            _ => {}
        }
    }
}

/// 写整个包；成功返回设备报的音色 id。`progress` 收 0 到 1。
pub async fn write_pack(
    transport: Arc<dyn Transport>,
    mut bus: broadcast::Receiver<DeviceMessage>,
    pack: Vec<u8>,
    progress: impl Fn(f32),
) -> Result<String, String> {
    voice_id_of(&pack).ok_or_else(|| "不是语音包".to_owned())?;
    send(&transport, begin_event(pack.len())).await?;
    wait_for(&mut bus, "voice.ready", None).await?;
    let total = pack.len().div_ceil(CHUNK_BYTES);
    for (seq, piece) in pack.chunks(CHUNK_BYTES).enumerate() {
        send(&transport, chunk_event(seq, piece)).await?;
        wait_for(&mut bus, "voice.ack", Some(seq as i64)).await?;
        progress((seq + 1) as f32 / total as f32);
    }
    send(&transport, end_event()).await?;
    let written = wait_for(&mut bus, "voice.written", None).await?;
    written
        .extra
        .get("voice")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "设备没有报告音色 id".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_voice_id_comes_from_the_pack_header() {
        let mut pack = vec![0_u8; 300];
        pack[0..4].copy_from_slice(b"VBVP");
        pack[16..28].copy_from_slice(b"wanwanxiaohe");
        assert_eq!(voice_id_of(&pack).as_deref(), Some("wanwanxiaohe"));
        pack[0] = b'X';
        assert_eq!(voice_id_of(&pack), None);
        assert_eq!(voice_id_of(&[]), None);
    }

    #[test]
    fn a_chunk_carries_its_sequence_crc_and_base64_within_the_line_limit() {
        let piece = vec![0xAB_u8; CHUNK_BYTES];
        let event = chunk_event(7, &piece);
        assert_eq!(event.extra["seq"], 7);
        assert_eq!(event.extra["crc"], crc32fast::hash(&piece));
        assert_eq!(event.extra["data"].as_str().map(str::len), Some(896));
        let frame = event.to_ndjson().expect("一块要能装进一行");
        assert!(frame.len() <= 1024, "{} 字节", frame.len());
    }
}
