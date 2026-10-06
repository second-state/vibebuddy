//! Writes a voice pack to the device over the serial protocol: stop-and-wait flow control, a CRC per block, the next block only after each receipt.
//! Protocol in docs/protocol.md, "Device maintenance".

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use vibebuddy_protocol::Event;
use tokio::sync::broadcast;

use crate::serial_transport::{DeviceMessage, Transport, TransportError};

/// Raw bytes per block: after base64 plus the JSON envelope it still fits the 1024-byte line limit.
pub const CHUNK_BYTES: usize = 672;
const HEADER_BYTES: usize = 256;
/// Upper bound for waiting on a device receipt. Erasing the 2 MB partition takes two or three seconds, and the final read-back check takes a while too.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// Reads the Character (or old voice) id from the header; None if it is neither kind of pack. Both
/// keep the id at the same place.
pub fn voice_id_of(pack: &[u8]) -> Option<String> {
    if pack.len() <= HEADER_BYTES || !matches!(&pack[0..4], b"VBVP" | b"VBCP") {
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
    // A full queue just means the device hasn't caught up yet; wait a bit and push again.
    for _ in 0..200 {
        match transport.send(frame.clone()) {
            Ok(()) => return Ok(()),
            Err(TransportError::QueueFull) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(TransportError::Closed) => return Err("serial worker has stopped".to_owned()),
        }
    }
    Err("device send queue stayed full".to_owned())
}

/// Waits for one specific device receipt. `voice.error` and a dropped link both count as failure.
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
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err("device message channel closed".to_owned()),
            Err(_) => return Err(format!("timed out waiting for {wanted}")),
        };
        match message {
            DeviceMessage::Disconnected => return Err("link lost".to_owned()),
            DeviceMessage::Event(event) if event.event == "voice.error" => {
                // `message` is a proper field of the protocol envelope, not part of extra.
                let detail = event.message.as_deref().unwrap_or("unknown error");
                return Err(format!("device refused: {detail}"));
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

/// Writes the whole pack; on success returns the voice id the device reports. `progress` receives 0 to 1.
pub async fn write_pack(
    transport: Arc<dyn Transport>,
    mut bus: broadcast::Receiver<DeviceMessage>,
    pack: Vec<u8>,
    progress: impl Fn(f32),
) -> Result<String, String> {
    voice_id_of(&pack).ok_or_else(|| "not a voice pack".to_owned())?;
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
        .ok_or_else(|| "device did not report a voice id".to_owned())
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
        pack[0..4].copy_from_slice(b"VBCP");
        assert_eq!(voice_id_of(&pack).as_deref(), Some("wanwanxiaohe"), "a Character pack");
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
        let frame = event.to_ndjson().expect("a chunk must fit in one line");
        assert!(frame.len() <= 1024, "{} bytes", frame.len());
    }
}
