//! The computer's side of the Relay (ADR-0012, docs/relay-protocol.md): reaching the box through
//! `vibebuddy-relay` when neither the cable nor the local network does.
//!
//! Once linked, a bridge task turns the Relay's `data` messages into the same NDJSON byte stream the
//! cable carries, so the serial worker's session loop runs over it unchanged. Firmware that sends
//! `link.noise` gets a Noise session, and every `data` line after the handshake is sealed (ADR-0015,
//! [`crate::noise_link`]). The bridge also sends WebSocket pings, gives the link up when the Relay goes
//! quiet, and ends the stream when the box is taken away.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

use crate::identity::Identity;
use crate::noise_link::{self, Memory, Session};

/// The Relay protocol version this daemon speaks (docs/relay-protocol.md).
const PROTOCOL_VERSION: u32 = 1;

/// The exact text the Relay counts as a sign of life and answers without waking.
const PING: &str = r#"{"t":"ping"}"#;

/// The Relay releases use, and the one every other build uses (docs/relay-protocol.md, "Which Relay").
pub const PRODUCTION_URL: &str = "wss://relay.korekore.ai";
pub const STAGING_URL: &str = "wss://relay-staging.korekore.ai";
/// A build CI made from a release tag (`VIBEBUDDY_RELEASE_TAG=1` at compile time). Builds from main and
/// from source use staging, so development never touches the production Relay.
pub fn release() -> bool {
    option_env!("VIBEBUDDY_RELEASE_TAG") == Some("1")
}
/// Pings go out this often, and a Relay silent this long is taken for gone (docs/relay-protocol.md).
const PING_EVERY: Duration = Duration::from_secs(30);
const DEAD_AFTER: Duration = Duration::from_secs(75);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
pub struct RelayConfig {
    pub identity: Arc<Identity>,
    /// Production or staging by build, or `ws://127.0.0.1:8787` under `wrangler dev`; the update manifest may
    /// move a release's.
    url: Arc<std::sync::RwLock<String>>,
    /// The update manifest may move the Relay: a release build on its compiled default.
    movable: bool,
}

impl RelayConfig {
    /// `fixed` is an address given by hand (`VIBEBUDDY_RELAY_URL`); None means the default for a release or
    /// for any other build, of which only a release's may be moved by the manifest.
    pub fn new(identity: Arc<Identity>, fixed: Option<String>, release: bool) -> Self {
        let default = if release { PRODUCTION_URL } else { STAGING_URL };
        let movable = release && fixed.is_none();
        let url = fixed.unwrap_or_else(|| default.to_owned());
        Self { identity, url: Arc::new(std::sync::RwLock::new(url)), movable }
    }

    pub fn url(&self) -> String {
        self.url.read().map(|url| url.clone()).unwrap_or_default()
    }

    /// The URL to give the box over the cable, so it uses the same Relay: only `wss://` and a host, the
    /// only kind it takes (docs/protocol.md, "Relay"). None for a local Relay such as `wrangler dev`.
    pub fn for_box(&self) -> Option<String> {
        let url = self.url();
        let host = url.strip_prefix("wss://")?.trim_end_matches('/');
        let valid = !host.is_empty() && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-');
        valid.then(|| format!("wss://{host}"))
    }

    /// The signed update manifest names the Relay: used from the next connection on. Only `wss://`, and never over
    /// an address given by hand.
    pub fn offer(&self, url: &str) {
        if !self.movable || !url.starts_with("wss://") {
            return;
        }
        if let Ok(mut current) = self.url.write()
            && *current != url
        {
            info!(%url, "the update manifest moves the Relay");
            *current = url.to_owned();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RelayError {
    /// Couldn't reach the Relay, or it went away.
    Unreachable(String),
    /// HTTP 429: too many connections for this box from here; wait before trying again.
    RateLimited,
    /// It refused us: `auth`, `not_paired` or another error code.
    Refused(String),
    /// Close 4005: the Relay no longer speaks this daemon's protocol version. Only an update helps.
    Upgrade,
}

impl RelayError {
    /// How long to wait before the next try.
    pub fn backoff(&self, attempt: u32) -> Duration {
        let base = match self {
            Self::RateLimited => Duration::from_secs(30),
            Self::Refused(_) => Duration::from_secs(60),
            Self::Unreachable(_) => Duration::from_secs(2),
            // Not again in this run: the next try would be refused the same way.
            Self::Upgrade => return Duration::from_secs(365 * 24 * 3600),
        };
        (base * 2u32.saturating_pow(attempt.min(5))).min(Duration::from_secs(300))
    }
}

/// A link that lasted this long was fine, and the next try starts the backoff over.
const LINK_WAS_FINE: Duration = Duration::from_secs(60);

/// When to try the Relay again after a link through it ended, and the attempt count to carry: soon after a link
/// that lasted, and backing off like an unreachable Relay after one that ended quickly.
pub fn after_link(lasted: Duration, attempts: u32) -> (Duration, u32) {
    if lasted >= LINK_WAS_FINE {
        return (RelayError::Unreachable(String::new()).backoff(0), 0);
    }
    (RelayError::Unreachable(String::new()).backoff(attempts), attempts.saturating_add(1))
}

pub fn box_url(base: &str, box_key: &[u8; 32]) -> String {
    format!("{}/v1/boxes/{}", base.trim_end_matches('/'), URL_SAFE_NO_PAD.encode(box_key))
}

pub fn signed_message(role: &str, box_id: &str, nonce: &str) -> Vec<u8> {
    format!("vibebuddy-relay-v1\n{role}\n{box_id}\n{nonce}").into_bytes()
}

/// TLS the way the rest of the daemon does it: rustls with ring and the system's certificate store.
fn connector() -> Result<Connector, RelayError> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| RelayError::Unreachable(error.to_string()))?
        .with_platform_verifier()
        .map_err(|error| RelayError::Unreachable(error.to_string()))?
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

/// How a link that finds the box busy waits on the same socket instead of reconnecting: the Relay pushes
/// `status` whenever the owner or the box changes, and a takeover goes out the moment the user asks.
pub struct Wait<'a> {
    /// Set when the user chose Take over; taken as it is sent.
    pub takeover: &'a AtomicBool,
    /// Told who has the box, each time the Relay says busy.
    pub busy: &'a mut (dyn FnMut(String, String) + Send),
    /// True when waiting no longer makes sense: the box turned up on the cable, or the Relay was turned off.
    pub give_up: &'a (dyn Fn() -> bool + Sync),
    /// Told, once linked, whenever the box goes off or comes back on the Relay (`status` box, or `linked` again).
    pub presence: Arc<dyn Fn(bool) + Send + Sync>,
    /// Which box has spoken Noise to this computer; None links in the clear unless the box opens a session.
    pub memory: Option<Memory>,
}

/// Connects, proves this computer's key, and asks for the box, waiting on the socket while someone else has
/// it. On success the returned stream carries the VibeBuddy Protocol as bytes, as the cable does.
pub async fn link(config: &RelayConfig, box_key: &[u8; 32], wait: Wait<'_>) -> Result<DuplexStream, RelayError> {
    let url = box_url(&config.url(), box_key);
    let connected = tokio_tungstenite::connect_async_tls_with_config(&url, None, false, Some(connector()?));
    let (mut socket, _) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, connected).await {
        Err(_) => return Err(RelayError::Unreachable("timed out".to_owned())),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status().as_u16() == 429 => {
            return Err(RelayError::RateLimited);
        }
        Ok(Err(error)) => return Err(RelayError::Unreachable(error.to_string())),
        Ok(Ok(connected)) => connected,
    };
    let presence = wait.presence.clone();
    let memory = wait.memory.clone();
    let result = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut socket, &config.identity, box_key)).await {
        Err(_) => Err(RelayError::Unreachable("handshake timed out".to_owned())),
        Ok(Err(error)) => Err(error),
        Ok(Ok(())) => wait_for_box(&mut socket, wait).await,
    };
    match result {
        Ok(()) => Ok(bridge(socket, presence, Noise { identity: config.identity.clone(), box_key: *box_key, memory })),
        Err(error) => {
            close_normally(&mut socket).await;
            Err(error)
        }
    }
}

/// Asks for the box and waits on the socket until it is ours.
async fn wait_for_box<S>(socket: &mut S, wait: Wait<'_>) -> Result<(), RelayError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let first = if wait.takeover.swap(false, Ordering::Relaxed) { "takeover" } else { "link" };
    send(socket, json!({"t": first})).await?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut pinged = tokio::time::Instant::now();
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                    match value["t"].as_str() {
                        Some("linked") => return Ok(()),
                        Some("busy") => {
                            let text = |name: &str| value[name].as_str().unwrap_or_default().to_owned();
                            (wait.busy)(text("owner"), text("reason"));
                        }
                        // The owner or the box changed: if nobody holds the box now, ask again. The Relay
                        // decides about the grace period, so a busy answer just means keep waiting.
                        Some("status") if value["owner"].is_null() || value["owner"]["linked"] == false => {
                            send(socket, json!({"t": "link"})).await?;
                        }
                        Some("error") if value["code"] == "upgrade" => return Err(RelayError::Upgrade),
                        Some("error") => return Err(RelayError::Refused(value["code"].as_str().unwrap_or("error").to_owned())),
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(frame))) => return Err(closed(frame.as_ref())),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(RelayError::Unreachable(error.to_string())),
                None => return Err(RelayError::Unreachable("closed".to_owned())),
            },
            _ = tick.tick() => {
                if wait.takeover.swap(false, Ordering::Relaxed) {
                    send(socket, json!({"t": "takeover"})).await?;
                }
                if (wait.give_up)() {
                    return Err(RelayError::Unreachable("no longer needed".to_owned()));
                }
                // The Relay counts this as a sign of life without waking for it.
                if pinged.elapsed() >= PING_EVERY {
                    socket.send(Message::text(PING)).await.map_err(|error| RelayError::Unreachable(error.to_string()))?;
                    pinged = tokio::time::Instant::now();
                }
            }
        }
    }
}

/// A close frame with 1000, so the Relay can tell a socket given up on purpose from a network drop.
async fn close_normally<S>(socket: &mut S)
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let frame = CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal, reason: "".into() };
    let _ = socket.send(Message::Close(Some(frame))).await;
    let _ = socket.close().await;
}

async fn handshake<S>(socket: &mut S, identity: &Identity, box_key: &[u8; 32]) -> Result<(), RelayError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let box_id = URL_SAFE_NO_PAD.encode(box_key);
    let challenge = expect(socket, &["challenge"]).await?;
    let nonce = challenge["nonce"].as_str().unwrap_or_default().to_owned();
    let sig = URL_SAFE_NO_PAD.encode(identity.sign(&signed_message("computer", &box_id, &nonce)));
    send(socket, json!({"t": "hello", "role": "computer", "key": identity.public_key(), "sig": sig, "v": PROTOCOL_VERSION})).await?;
    expect(socket, &["welcome"]).await?;
    Ok(())
}

/// Reads until one of `wanted` arrives, skipping `status` and pings; an `error` or a close ends it.
async fn expect<S>(socket: &mut S, wanted: &[&str]) -> Result<Value, RelayError>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                let kind = value["t"].as_str().unwrap_or_default();
                if wanted.contains(&kind) {
                    return Ok(value);
                }
                if kind == "error" && value["code"] == "upgrade" {
                    return Err(RelayError::Upgrade);
                }
                if kind == "error" {
                    return Err(RelayError::Refused(value["code"].as_str().unwrap_or("error").to_owned()));
                }
            }
            Some(Ok(Message::Close(frame))) => return Err(closed(frame.as_ref())),
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(RelayError::Unreachable(error.to_string())),
            None => return Err(RelayError::Unreachable("closed".to_owned())),
        }
    }
}

fn closed(frame: Option<&CloseFrame>) -> RelayError {
    match frame.map(|frame| u16::from(frame.code)) {
        Some(4001) => RelayError::Refused("auth".to_owned()),
        Some(4003) => RelayError::Refused("not_paired".to_owned()),
        Some(1008) => RelayError::RateLimited,
        // Silent too long, as a sleeping laptop is: a drop like any other, and the box may still be ours.
        Some(4004) => RelayError::Unreachable("silent".to_owned()),
        Some(4005) => RelayError::Upgrade,
        Some(code) => RelayError::Unreachable(format!("closed with {code}")),
        None => RelayError::Unreachable("closed".to_owned()),
    }
}

async fn send<S>(socket: &mut S, value: Value) -> Result<(), RelayError>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    socket.send(Message::text(value.to_string())).await.map_err(|error| RelayError::Unreachable(error.to_string()))
}

/// What the bridge needs for a session with the box.
struct Noise {
    identity: Arc<Identity>,
    box_key: [u8; 32],
    memory: Option<Memory>,
}

/// Where the bridge is with the box's session.
enum Mode {
    /// Firmware that doesn't speak Noise: lines go as they are.
    Clear,
    /// Waiting for the box's `link.noise`; nothing in the clear is believed or sent.
    Waiting,
    Handshaking(Box<noise_link::Handshake>),
    Sealed(Session),
}

/// Lines held while a session comes up; past this the oldest go, as on a full cable.
const HELD_LINES: usize = 64;

/// Runs the linked socket behind a byte stream: lines written to the stream go out as `data`, `data`
/// from the box comes back as lines, sealed once the box opens a session. The stream ends when the box
/// is taken away, the Relay closes, a sealed line doesn't open, or the Relay goes quiet for [`DEAD_AFTER`].
fn bridge(socket: Socket, presence: Arc<dyn Fn(bool) + Send + Sync>, noise: Noise) -> DuplexStream {
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    tokio::spawn(run_bridge(socket, theirs, presence, noise));
    ours
}

async fn run_bridge<S>(socket: S, stream: DuplexStream, presence: Arc<dyn Fn(bool) + Send + Sync>, noise: Noise)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    let (mut sink, mut source) = socket.split();
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = tokio::io::BufReader::new(read).lines();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heard = tokio::time::Instant::now();
    // The Relay links a computer even while the box is away; its first status says whether it is there.
    let mut box_here = true;
    let knows_noise = || noise.memory.as_ref().is_some_and(|memory| memory.knows(&noise.box_key));
    let mut mode = if knows_noise() { Mode::Waiting } else { Mode::Clear };
    let mut held: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let hello = serde_json::to_string(&vibebuddy_protocol::Event::named("device.hello")).unwrap_or_default();
    let data = |line: &str| Message::text(json!({"t": "data", "line": line}).to_string());
    loop {
        tokio::select! {
            message = source.next() => {
                heard = tokio::time::Instant::now();
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                        match value["t"].as_str() {
                            Some("data") => {
                                let Some(line) = value["line"].as_str() else { continue };
                                // The box opens a session whenever a computer links to it, so this also starts
                                // a new one after the box comes back.
                                if noise_link::opener_key(line) == Some(noise.box_key) {
                                    match noise_link::start(&noise.identity, &noise.box_key, b"{}") {
                                        Ok((handshake, first)) => {
                                            mode = Mode::Handshaking(Box::new(handshake));
                                            if sink.send(data(&first)).await.is_err() {
                                                break;
                                            }
                                        }
                                        Err(error) => {
                                            warn!(%error, "couldn't start a session with the box");
                                            break;
                                        }
                                    }
                                    continue;
                                }
                                let plain = match std::mem::replace(&mut mode, Mode::Waiting) {
                                    Mode::Clear => {
                                        mode = Mode::Clear;
                                        line.to_owned()
                                    }
                                    Mode::Waiting => continue,
                                    Mode::Handshaking(handshake) => {
                                        let Ok(mut session) = handshake.finish(line) else {
                                            warn!("the box didn't prove its key through the Relay");
                                            break;
                                        };
                                        if let Some(memory) = &noise.memory {
                                            memory.learned(&noise.box_key);
                                        }
                                        // A new session is a new start, as on a new cable connection: hello first,
                                        // then whatever waited for it, less a hello of its own.
                                        let mut failed = false;
                                        let waited: Vec<String> = held.drain(..).filter(|line| *line != hello).collect();
                                        for line in std::iter::once(hello.clone()).chain(waited) {
                                            let sent = match session.seal(line.as_bytes()) {
                                                Ok(sealed) => sink.send(data(&sealed)).await.is_ok(),
                                                Err(_) => false,
                                            };
                                            if !sent {
                                                failed = true;
                                                break;
                                            }
                                        }
                                        if failed {
                                            break;
                                        }
                                        mode = Mode::Sealed(session);
                                        continue;
                                    }
                                    Mode::Sealed(mut session) => match session.open(line) {
                                        Ok(plain) => {
                                            mode = Mode::Sealed(session);
                                            String::from_utf8_lossy(&plain).into_owned()
                                        }
                                        Err(error) => {
                                            warn!(%error, "ending the session with the box");
                                            break;
                                        }
                                    },
                                };
                                if write.write_all(format!("{plain}\n").as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            // Linked again while linked: the box reconnected to the Relay (it restarted, say), so
                            // the session starts over with hello, as on a new cable connection. A box that speaks
                            // Noise opens a new session first, and hello goes out sealed once it is up.
                            Some("linked") => {
                                if !box_here {
                                    box_here = true;
                                    presence(true);
                                }
                                if matches!(mode, Mode::Clear) && !knows_noise() {
                                    if sink.send(data(&hello)).await.is_err() {
                                        break;
                                    }
                                } else {
                                    mode = Mode::Waiting;
                                }
                            }
                            Some("unlinked") => {
                                info!(reason = value["reason"].as_str().unwrap_or("?"), "the Relay link ended");
                                break;
                            }
                            Some("error") => warn!(code = value["code"].as_str().unwrap_or("?"), "the Relay reported an error"),
                            // The box went away (its power, its Wi-Fi) while this computer stays linked: the link
                            // is down until the Relay says linked again.
                            Some("status") => {
                                debug!(%value, "Relay status");
                                if value["box"] == false && box_here {
                                    box_here = false;
                                    presence(false);
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        info!(reason = ?closed(frame.as_ref()), "the Relay closed the link");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        warn!(%error, "the Relay link failed");
                        break;
                    }
                    None => break,
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        let message = match &mut mode {
                            Mode::Clear => data(&line),
                            Mode::Sealed(session) => match session.seal(line.as_bytes()) {
                                Ok(sealed) => data(&sealed),
                                Err(_) => break,
                            },
                            Mode::Waiting | Mode::Handshaking(_) => {
                                if held.len() == HELD_LINES {
                                    held.pop_front();
                                }
                                held.push_back(line);
                                continue;
                            }
                        };
                        if sink.send(message).await.is_err() {
                            break;
                        }
                    }
                    // The session let go of the stream: give the box up.
                    _ => {
                        let _ = sink.send(Message::text(json!({"t": "unlink"}).to_string())).await;
                        close_normally(&mut sink).await;
                        return;
                    }
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() >= DEAD_AFTER {
                    warn!("the Relay went quiet, reconnecting");
                    break;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    close_normally(&mut sink).await;
    // Dropping the write half ends the stream for the session, which then reconnects.
    let _ = write.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio_tungstenite::accept_async;

    fn identity() -> Identity {
        let mut suffix = [0u8; 4];
        getrandom::fill(&mut suffix).unwrap();
        let dir = std::env::temp_dir().join(format!("vibebuddy-relay-test-{}-{}", std::process::id(), u32::from_le_bytes(suffix)));
        Identity::load_or_create(&dir.join("identity"), "test".to_owned()).unwrap()
    }

    /// A Relay that checks the hello, then answers link with `answer` and echoes data back in upper case.
    async fn fake_relay(answer: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let say = |value: Value| Message::text(value.to_string());
            socket.send(say(json!({"t": "challenge", "nonce": "abc"}))).await.unwrap();
            let Some(Ok(Message::Text(hello))) = socket.next().await else { return };
            let hello: Value = serde_json::from_str(&hello).unwrap();
            let key = URL_SAFE_NO_PAD.decode(hello["key"].as_str().unwrap()).unwrap();
            let sig = URL_SAFE_NO_PAD.decode(hello["sig"].as_str().unwrap()).unwrap();
            let verifying = ed25519_dalek::VerifyingKey::from_bytes(&key.try_into().unwrap()).unwrap();
            let box_id = URL_SAFE_NO_PAD.encode([5u8; 32]);
            let valid = verifying
                .verify_strict(&signed_message("computer", &box_id, "abc"), &ed25519_dalek::Signature::from_bytes(&sig.try_into().unwrap()))
                .is_ok();
            if !valid {
                socket.send(say(json!({"t": "error", "code": "auth"}))).await.unwrap();
                return;
            }
            socket.send(say(json!({"t": "welcome"}))).await.unwrap();
            socket.send(say(json!({"t": "status", "box": true, "owner": null}))).await.unwrap();
            let Some(Ok(Message::Text(_link))) = socket.next().await else { return };
            match answer {
                // Linked; then the box loses power and comes back.
                "away" => {
                    socket.send(say(json!({"t": "linked"}))).await.unwrap();
                    socket.send(say(json!({"t": "status", "box": false, "owner": {"key": "k", "name": "n", "via": "relay", "linked": true}}))).await.unwrap();
                    socket.send(say(json!({"t": "linked"}))).await.unwrap();
                    socket.send(say(json!({"t": "unlinked", "reason": "takeover"}))).await.unwrap();
                    while socket.next().await.is_some() {}
                }
                "busy-forever" => {
                    socket.send(say(json!({"t": "busy", "reason": "linked", "owner": "omarchy"}))).await.unwrap();
                    while socket.next().await.is_some() {}
                }
                // Busy first; then the owner leaves, and the daemon asks again on the same socket.
                "busy" => {
                    socket.send(say(json!({"t": "busy", "reason": "linked", "owner": "omarchy"}))).await.unwrap();
                    socket.send(say(json!({"t": "status", "box": true, "owner": null}))).await.unwrap();
                    while let Some(Ok(Message::Text(text))) = socket.next().await {
                        if serde_json::from_str::<Value>(&text).unwrap()["t"] == "link" {
                            break;
                        }
                    }
                    socket.send(say(json!({"t": "linked"}))).await.unwrap();
                    while socket.next().await.is_some() {}
                }
                _ => {
                    socket.send(say(json!({"t": "linked"}))).await.unwrap();
                    while let Some(Ok(message)) = socket.next().await {
                        let Message::Text(text) = message else { continue };
                        let value: Value = serde_json::from_str(&text).unwrap();
                        if value["t"] == "data" {
                            let line = value["line"].as_str().unwrap();
                            if line.contains("device.hello") && line.contains("\"version\"") && !line.contains("VERSION") && answer == "restart" {
                                // The bridge said hello again after the box came back: end the test there.
                                socket.send(say(json!({"t": "data", "line": "REHELLO"}))).await.unwrap();
                                socket.send(say(json!({"t": "unlinked", "reason": "takeover"}))).await.unwrap();
                                continue;
                            }
                            socket.send(say(json!({"t": "data", "line": line.to_uppercase()}))).await.unwrap();
                            if answer == "restart" {
                                socket.send(say(json!({"t": "linked"}))).await.unwrap();
                            } else {
                                socket.send(say(json!({"t": "unlinked", "reason": "takeover"}))).await.unwrap();
                            }
                        }
                    }
                }
            }
        });
        format!("ws://{address}")
    }

    /// Links without waiting for anything: no takeover, busy ignored, never gives up.
    async fn link_now(config: &RelayConfig) -> Result<DuplexStream, RelayError> {
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        link(config, &[5; 32], Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence: Arc::new(|_| {}), memory: None }).await
    }

    #[tokio::test]
    async fn lines_go_through_the_relay_both_ways_until_the_box_is_taken() {
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("linked").await), false);
        let mut stream = link_now(&config).await.unwrap();
        stream.write_all(b"{\"version\":1,\"event\":\"device.hello\"}\n").await.unwrap();
        let mut received = String::new();
        stream.read_to_string(&mut received).await.unwrap();
        assert_eq!(received, "{\"VERSION\":1,\"EVENT\":\"DEVICE.HELLO\"}\n", "and then the stream ends on unlinked");
    }

    #[tokio::test]
    async fn the_box_coming_back_starts_the_session_over() {
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("restart").await), false);
        let mut stream = link_now(&config).await.unwrap();
        stream.write_all(b"{\"version\":1,\"event\":\"x\"}\n").await.unwrap();
        let mut received = String::new();
        stream.read_to_string(&mut received).await.unwrap();
        assert!(received.ends_with("REHELLO\n"), "hello went out again after linked: {received}");
    }

    #[tokio::test]
    async fn a_busy_link_waits_on_the_socket_and_gets_the_box_when_it_frees_up() {
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("busy").await), false);
        let takeover = AtomicBool::new(false);
        let owners = std::sync::Mutex::new(Vec::new());
        let mut busy = |owner: String, reason: String| owners.lock().unwrap().push((owner, reason));
        let linked = link(&config, &[5; 32], Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence: Arc::new(|_| {}), memory: None }).await;
        assert!(linked.is_ok());
        assert_eq!(owners.lock().unwrap().as_slice(), [("omarchy".to_owned(), "linked".to_owned())]);
    }

    #[tokio::test]
    async fn the_box_going_away_and_coming_back_is_reported() {
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("away").await), false);
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        let presence: Arc<dyn Fn(bool) + Send + Sync> = Arc::new(move |here| record.lock().unwrap().push(here));
        let mut stream = link(&config, &[5; 32], Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence, memory: None }).await.unwrap();
        let mut rest = String::new();
        stream.read_to_string(&mut rest).await.unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), [false, true]);
    }

    #[tokio::test]
    async fn waiting_stops_when_no_longer_needed() {
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("busy-forever").await), false);
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        let result = link(&config, &[5; 32], Wait { takeover: &takeover, busy: &mut busy, give_up: &|| true, presence: Arc::new(|_| {}), memory: None }).await;
        assert!(matches!(result, Err(RelayError::Unreachable(_))));
    }

    #[test]
    fn links_that_end_quickly_back_off() {
        // Thirty quick ends in a row stay well under the Relay's 30 new connections a minute.
        let (mut total, mut attempts) = (Duration::ZERO, 0);
        for _ in 0..30 {
            let (wait, next) = after_link(Duration::from_millis(200), attempts);
            total += wait + Duration::from_millis(200);
            attempts = next;
        }
        assert!(total > Duration::from_secs(60 * 30 / 30), "{total:?}");
        assert_eq!(after_link(Duration::from_secs(600), 7), (Duration::from_secs(2), 0), "a long link starts over");
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let error = RelayError::Unreachable(String::new());
        assert_eq!(error.backoff(0), Duration::from_secs(2));
        assert_eq!(error.backoff(1), Duration::from_secs(4));
        assert_eq!(RelayError::RateLimited.backoff(9), Duration::from_secs(300));
    }

    #[test]
    fn the_manifest_moves_a_releases_relay_unless_it_was_given_by_hand() {
        let config = RelayConfig::new(Arc::new(identity()), None, true);
        assert_eq!(config.url(), "wss://relay.korekore.ai");
        config.offer("ws://plain.example");
        assert_eq!(config.url(), PRODUCTION_URL, "never off TLS");
        config.offer("wss://relay2.korekore.ai");
        assert_eq!(config.url(), "wss://relay2.korekore.ai");
        let fixed = RelayConfig::new(Arc::new(identity()), Some("ws://127.0.0.1:8787".to_owned()), true);
        fixed.offer("wss://relay2.korekore.ai");
        assert_eq!(fixed.url(), "ws://127.0.0.1:8787");
    }

    #[test]
    fn other_builds_use_staging_and_keep_it() {
        let config = RelayConfig::new(Arc::new(identity()), None, false);
        assert_eq!(config.url(), STAGING_URL);
        config.offer("wss://relay.korekore.ai");
        assert_eq!(config.url(), STAGING_URL, "the manifest is the releases'");
    }

    #[test]
    fn the_box_is_given_only_a_wss_host() {
        let config = |url: &str| RelayConfig::new(Arc::new(identity()), Some(url.to_owned()), false);
        assert_eq!(config("wss://relay-staging.korekore.ai/").for_box().as_deref(), Some(STAGING_URL));
        assert_eq!(config("ws://127.0.0.1:8787").for_box(), None);
        assert_eq!(config("wss://127.0.0.1:8787").for_box(), None);
    }

    #[test]
    fn an_old_protocol_is_not_retried() {
        assert!(RelayError::Upgrade.backoff(0) > Duration::from_secs(300 * 24 * 3600));
        assert_eq!(closed(Some(&CloseFrame { code: 4005.into(), reason: "".into() })), RelayError::Upgrade);
    }

    #[test]
    fn the_url_names_the_box() {
        assert_eq!(box_url("wss://relay.korekore.ai/", &[0; 32]), "wss://relay.korekore.ai/v1/boxes/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    }

    /// A Relay with a box behind it that speaks Noise, played by the firmware's own code: it opens a session
    /// when linked, then answers every sealed line with the line in upper case, sealed. What it saw in the clear
    /// goes to `clear`.
    async fn noise_relay(computer: [u8; 32], clear: Arc<std::sync::Mutex<Vec<String>>>) -> (String, [u8; 32]) {
        use vibebuddy_firmware_core::link::Link;
        use vibebuddy_firmware_core::noise;
        let mut link = Link::new([1; 32]);
        link.pair(computer, "test").unwrap();
        let box_key = link.public_key();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let say = |value: Value| Message::text(value.to_string());
            socket.send(say(json!({"t": "challenge", "nonce": "abc"}))).await.unwrap();
            let _hello = socket.next().await;
            socket.send(say(json!({"t": "welcome"}))).await.unwrap();
            let _link = socket.next().await;
            socket.send(say(json!({"t": "linked"}))).await.unwrap();
            socket.send(say(json!({"t": "data", "line": noise::opener(&link.public_key())}))).await.unwrap();
            let mut session = None;
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else { continue };
                let value: Value = serde_json::from_str(&text).unwrap();
                let Some(line) = value["line"].as_str() else { continue };
                match &mut session {
                    // As the firmware does, anything before the handshake is never acted on.
                    None => match noise::accept(&link, [4; 32], line.as_bytes()) {
                        Ok(accepted) => {
                            socket.send(say(json!({"t": "data", "line": accepted.reply}))).await.unwrap();
                            session = Some((accepted.sealer, accepted.opener));
                        }
                        Err(_) => clear.lock().unwrap().push(line.to_owned()),
                    },
                    Some((sealer, opener)) => {
                        let Some(plain) = opener.open(line.as_bytes()) else {
                            clear.lock().unwrap().push(line.to_owned());
                            continue;
                        };
                        let upper = String::from_utf8(plain).unwrap().to_uppercase();
                        socket.send(say(json!({"t": "data", "line": sealer.seal(upper.as_bytes()).unwrap()}))).await.unwrap();
                        if upper.contains("BYE") {
                            socket.send(say(json!({"t": "unlinked", "reason": "takeover"}))).await.unwrap();
                        }
                    }
                }
            }
        });
        (format!("ws://{address}"), box_key)
    }

    fn computer_key(identity: &Identity) -> [u8; 32] {
        URL_SAFE_NO_PAD.decode(identity.public_key()).unwrap().try_into().unwrap()
    }

    #[tokio::test]
    async fn a_box_that_speaks_noise_gets_every_line_sealed() {
        let identity = Arc::new(identity());
        let clear = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (url, box_key) = noise_relay(computer_key(&identity), clear.clone()).await;
        let config = RelayConfig::new(identity, Some(url), false);
        let dir = std::env::temp_dir().join(format!("vibebuddy-relay-noise-{}", std::process::id()));
        let memory = Memory::new(&dir);
        memory.learned(&box_key);
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        let wait = Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence: Arc::new(|_| {}), memory: Some(memory.clone()) };
        let mut stream = link(&config, &box_key, wait).await.unwrap();
        // Written before the session is up: held, then sealed after the hello the bridge sends first.
        stream.write_all(b"{\"version\":1,\"event\":\"bye\"}\n").await.unwrap();
        let mut received = String::new();
        stream.read_to_string(&mut received).await.unwrap();
        assert_eq!(received, "{\"VERSION\":1,\"EVENT\":\"DEVICE.HELLO\"}\n{\"VERSION\":1,\"EVENT\":\"BYE\"}\n");
        assert!(clear.lock().unwrap().is_empty(), "nothing went in the clear: {:?}", clear.lock().unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_box_not_yet_known_to_speak_noise_still_gets_a_session_and_is_remembered() {
        let identity = Arc::new(identity());
        let (url, box_key) = noise_relay(computer_key(&identity), Arc::new(std::sync::Mutex::new(Vec::new()))).await;
        let config = RelayConfig::new(identity, Some(url), false);
        let dir = std::env::temp_dir().join(format!("vibebuddy-relay-learn-{}", std::process::id()));
        let memory = Memory::new(&dir);
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        let wait = Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence: Arc::new(|_| {}), memory: Some(memory.clone()) };
        let stream = link(&config, &box_key, wait).await.unwrap();
        let mut lines = tokio::io::BufReader::new(stream).lines();
        assert_eq!(lines.next_line().await.unwrap().unwrap(), "{\"VERSION\":1,\"EVENT\":\"DEVICE.HELLO\"}");
        assert!(memory.knows(&box_key), "never in the clear again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_box_that_spoke_noise_before_is_never_believed_in_the_clear() {
        // An old-style Relay echoing in the clear, as one that hid the box's link.noise would.
        let config = RelayConfig::new(Arc::new(identity()), Some(fake_relay("linked").await), false);
        let dir = std::env::temp_dir().join(format!("vibebuddy-relay-downgrade-{}", std::process::id()));
        let memory = Memory::new(&dir);
        memory.learned(&[5; 32]);
        let takeover = AtomicBool::new(false);
        let mut busy = |_: String, _: String| {};
        let wait = Wait { takeover: &takeover, busy: &mut busy, give_up: &|| false, presence: Arc::new(|_| {}), memory: Some(memory) };
        let mut stream = link(&config, &[5; 32], wait).await.unwrap();
        stream.write_all(b"{\"version\":1,\"event\":\"device.hello\"}\n").await.unwrap();
        let mut received = String::new();
        let read = tokio::time::timeout(Duration::from_millis(500), stream.read_to_string(&mut received)).await;
        assert!(read.is_err() || received.is_empty(), "nothing in the clear came through: {received}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
