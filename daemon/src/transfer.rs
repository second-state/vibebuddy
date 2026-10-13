//! Sending a firmware image to the box over any link, in chunks, resumably (ADR-0016, docs/protocol.md
//! "Firmware over the link"). The image goes as `xfer.chunk` lines of at most [`CHUNK_BYTES`], up to [`WINDOW`]
//! of them unacknowledged; the box acknowledges each with the next offset it expects. When the link drops or
//! moves (the Relay to the cable, say), the daemon waits for it to come back and asks again with `xfer.begin`,
//! and the box answers with where it got to.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use vibebuddy_protocol::Event;

use crate::serial_transport::{DeviceMessage, Transport};
use crate::voice_writer::send;

/// Image bytes per chunk: as base64 inside its line about 4.2 KB, under the box's 4608.
pub const CHUNK_BYTES: usize = 3072;
/// Chunks sent ahead of the acknowledgements: enough to keep the Relay busy at 130-250 ms a round trip and the box
/// writing; [`PACE`] keeps the rate down.
const WINDOW: usize = 16;
/// At most one chunk this often, 40 a second: under the Relay's 50 messages a second for good, with room for the
/// heartbeat (docs/relay-protocol.md). About 120 KB/s, more than the box takes over any link.
const PACE: Duration = Duration::from_millis(25);
/// The box erases what the image needs before it answers `xfer.begin`: up to 4 MB, a block per main-loop pass.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// No answer to a chunk for this long means it went missing: ask where the box got to.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
/// The box reads 4 MB back to check it before it answers `xfer.end`.
const END_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a dropped link may take to come back before the update gives up.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(180);
/// Times the transfer may start over from where the box got to.
const RESUMES: u32 = 10;

/// Why sending stopped.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    /// The link dropped or moved, or the box went quiet: resume from where it got to.
    Resume,
    /// The box refused, with its code; or something that resuming won't fix.
    Fail(String),
}

fn event(name: &str, fields: serde_json::Value) -> Event {
    let mut event = Event::named(name);
    if let serde_json::Value::Object(map) = fields {
        event.extra = map.into_iter().collect();
    }
    event
}

pub fn begin_event(size: usize, sha256: &str) -> Event {
    event("xfer.begin", serde_json::json!({"kind": "firmware", "size": size, "sha256": sha256}))
}

pub fn chunk_event(offset: usize, piece: &[u8]) -> Event {
    event(
        "xfer.chunk",
        serde_json::json!({
            "offset": offset,
            "crc": crc32fast::hash(piece),
            "data": base64::engine::general_purpose::STANDARD.encode(piece),
        }),
    )
}

/// The box's next answer about the transfer: `xfer.ready`/`xfer.ack` with an offset, `xfer.done`, or a stop.
async fn answer(bus: &mut broadcast::Receiver<DeviceMessage>, timeout: Duration) -> Result<(String, Option<usize>), Stop> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let message = match tokio::time::timeout_at(deadline, bus.recv()).await {
            Ok(Ok(message)) => message,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err(Stop::Fail("device message channel closed".to_owned())),
            Err(_) => return Err(Stop::Resume),
        };
        match message {
            DeviceMessage::Disconnected | DeviceMessage::Connected { .. } | DeviceMessage::ConnectedNetwork { .. } | DeviceMessage::ConnectedRelay { .. } => {
                return Err(Stop::Resume);
            }
            DeviceMessage::Event(event) if event.event == "xfer.error" => {
                let code = event.extra.get("code").and_then(|value| value.as_str()).unwrap_or("error");
                return Err(Stop::Fail(format!("the box refused the image: {code}")));
            }
            DeviceMessage::Event(event) if event.event.starts_with("xfer.") => {
                let offset = event.extra.get("offset").and_then(serde_json::Value::as_u64).map(|offset| offset as usize);
                return Ok((event.event, offset));
            }
            _ => {}
        }
    }
}

/// Waits for the link to come back after it dropped; true once it has.
async fn reconnected(bus: &mut broadcast::Receiver<DeviceMessage>) -> bool {
    let deadline = tokio::time::Instant::now() + RECONNECT_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, bus.recv()).await {
            Ok(Ok(DeviceMessage::Connected { .. } | DeviceMessage::ConnectedNetwork { .. } | DeviceMessage::ConnectedRelay { .. })) => return true,
            Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return false,
        }
    }
}

/// Sends the image from wherever the box says it got to, until the box has it all.
async fn send_from_box(
    transport: &Arc<dyn Transport>,
    bus: &mut broadcast::Receiver<DeviceMessage>,
    image: &[u8],
    sha256: &str,
    progress: &impl Fn(f32),
) -> Result<(), Stop> {
    send(transport, begin_event(image.len(), sha256)).await.map_err(|_| Stop::Resume)?;
    let mut acked = loop {
        if let (name, Some(offset)) = answer(bus, READY_TIMEOUT).await?
            && name == "xfer.ready"
        {
            break offset;
        }
    };
    let mut next = acked;
    let mut pace = tokio::time::interval(PACE);
    pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    while acked < image.len() {
        while next < image.len() && next < acked + WINDOW * CHUNK_BYTES {
            pace.tick().await;
            let end = (next + CHUNK_BYTES).min(image.len());
            send(transport, chunk_event(next, &image[next..end])).await.map_err(|_| Stop::Resume)?;
            next = end;
        }
        if let (name, Some(offset)) = answer(bus, STEP_TIMEOUT).await?
            && name == "xfer.ack"
        {
            acked = acked.max(offset);
            progress(acked as f32 / image.len() as f32);
        }
    }
    Ok(())
}

/// Sends the whole image and has the box check it and make it boot next; the box then restarts. `progress`
/// receives 0 to 1 as the box acknowledges, and `resuming` is told when the transfer waits for the link.
pub async fn send_image(
    transport: Arc<dyn Transport>,
    mut bus: broadcast::Receiver<DeviceMessage>,
    image: Vec<u8>,
    progress: impl Fn(f32),
    resuming: impl Fn(),
) -> Result<(), String> {
    let sha256: String = Sha256::digest(&image).iter().map(|byte| format!("{byte:02x}")).collect();
    let mut resumes = 0;
    loop {
        match send_from_box(&transport, &mut bus, &image, &sha256, &progress).await {
            Ok(()) => {
                send(&transport, Event::named("xfer.end")).await?;
                match answer(&mut bus, END_TIMEOUT).await {
                    Ok((name, _)) if name == "xfer.done" => return Ok(()),
                    Ok((name, _)) => return Err(format!("unexpected {name} at the end")),
                    // The box keeps what it has: asking again carries on, and ends again.
                    Err(Stop::Resume) => {}
                    Err(Stop::Fail(why)) => return Err(why),
                }
            }
            Err(Stop::Fail(why)) => return Err(why),
            Err(Stop::Resume) => {}
        }
        resumes += 1;
        if resumes > RESUMES {
            return Err("the link kept dropping".to_owned());
        }
        resuming();
        // A link that moved is already back; one that dropped gets a while to come back.
        let connected = transport_connected(&mut bus).await;
        if !connected && !reconnected(&mut bus).await {
            return Err("the box didn't come back".to_owned());
        }
    }
}

/// Drains what the bus already holds; true if the latest word about the link is that it is up.
async fn transport_connected(bus: &mut broadcast::Receiver<DeviceMessage>) -> bool {
    let mut connected = None;
    while let Ok(message) = bus.try_recv() {
        match message {
            DeviceMessage::Disconnected => connected = Some(false),
            DeviceMessage::Connected { .. } | DeviceMessage::ConnectedNetwork { .. } | DeviceMessage::ConnectedRelay { .. } => connected = Some(true),
            _ => {}
        }
    }
    connected.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::serial_transport::TransportError;

    /// Plays the box: takes chunks in order, acknowledges each with the next offset, and on xfer.begin
    /// answers where it got to. `drop_after` chunks in, it drops the link once and loses what was in flight.
    struct FakeBox {
        bus: broadcast::Sender<DeviceMessage>,
        state: Mutex<(usize, Vec<u8>, Option<usize>)>,
    }

    impl Transport for FakeBox {
        fn send(&self, frame: Vec<u8>) -> Result<(), TransportError> {
            let value: serde_json::Value = serde_json::from_slice(&frame).unwrap();
            let reply = |name: &str, offset: Option<usize>| {
                let mut event = Event::named(name);
                if let Some(offset) = offset {
                    event.extra.insert("offset".to_owned(), offset.into());
                }
                let _ = self.bus.send(DeviceMessage::Event(event));
            };
            let mut state = self.state.lock().unwrap();
            let (next, written, drop_after) = &mut *state;
            match value["event"].as_str().unwrap() {
                "xfer.begin" => reply("xfer.ready", Some(*next)),
                "xfer.chunk" => {
                    if *drop_after == Some(0) {
                        *drop_after = None;
                        let _ = self.bus.send(DeviceMessage::Disconnected);
                        let _ = self.bus.send(DeviceMessage::ConnectedRelay { url: "wss://relay".to_owned() });
                        return Ok(());
                    }
                    if let Some(count) = drop_after {
                        *count -= 1;
                    }
                    let offset = value["offset"].as_u64().unwrap() as usize;
                    if offset == *next {
                        let data = base64::engine::general_purpose::STANDARD.decode(value["data"].as_str().unwrap()).unwrap();
                        assert_eq!(value["crc"].as_u64().unwrap() as u32, crc32fast::hash(&data));
                        written.extend_from_slice(&data);
                        *next += data.len();
                    }
                    reply("xfer.ack", Some(*next));
                }
                "xfer.end" => reply("xfer.done", None),
                other => panic!("unexpected {other}"),
            }
            Ok(())
        }
    }

    async fn run(image: Vec<u8>, drop_after: Option<usize>) -> (Result<(), String>, Vec<u8>, usize) {
        let (bus, receiver) = broadcast::channel(1024);
        let fake = Arc::new(FakeBox { bus, state: Mutex::new((0, Vec::new(), drop_after)) });
        let resumed = Arc::new(Mutex::new(0));
        let counter = resumed.clone();
        let result = send_image(fake.clone(), receiver, image, |_| {}, move || *counter.lock().unwrap() += 1).await;
        let written = std::mem::take(&mut fake.state.lock().unwrap().1);
        let resumed = *resumed.lock().unwrap();
        (result, written, resumed)
    }

    #[tokio::test]
    async fn the_whole_image_arrives_in_order() {
        let image: Vec<u8> = (0..20_000).map(|index| (index * 31) as u8).collect();
        let (result, written, resumed) = run(image.clone(), None).await;
        assert_eq!(result, Ok(()));
        assert_eq!(written, image);
        assert_eq!(resumed, 0);
    }

    #[tokio::test]
    async fn a_dropped_link_resumes_where_the_box_got_to() {
        let image: Vec<u8> = (0..30_000).map(|index| (index * 17) as u8).collect();
        let (result, written, resumed) = run(image.clone(), Some(3)).await;
        assert_eq!(result, Ok(()));
        assert_eq!(written, image, "nothing written twice, nothing missing");
        assert_eq!(resumed, 1);
    }

    #[test]
    fn a_full_chunk_fits_the_boxs_line_limit() {
        let frame = chunk_event(4_000_000, &vec![0xAB; CHUNK_BYTES]).to_ndjson().unwrap();
        assert!(frame.len() <= 4608, "{} bytes", frame.len());
    }
}
