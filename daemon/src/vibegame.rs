//! The computer's side of the VibeGame (ADR-0025, docs/vibegame.md): the game handheld that shows the buddy
//! beside the box without taking the link.
//!
//! The daemon browses for `_vibebuddy-vibegame._tcp` all the time, pairs with a VibeGame when the user types the
//! code it shows (Noise XX), and then connects to every paired VibeGame it can reach (Noise IK, as for the box).
//! Each connected VibeGame gets a copy of what the box gets from the same snapshot, minus what looks after a box,
//! with `quiet` set while a box is linked. What the VibeGame sends back (K2 and a picked card) goes to the daemon
//! as [`Inbound`], never into the box's link (ADR-0011).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use vibebuddy_protocol::Event;

use crate::identity::Identity;
use crate::lan::{self, Discovery, LinkError};
use crate::noise_link;

/// The DNS-SD service type a VibeGame answers for. Not the box's, so box discovery never picks one up.
pub const SERVICE_TYPE: &str = "_vibebuddy-vibegame._tcp.local.";
/// Fixes one VibeGame address (`host:port`) and turns discovery off, for QEMU and tests.
pub const ADDRESS_VARIABLE: &str = "VIBEBUDDY_VIBEGAME_ADDR";
const PAIR_PROLOGUE: &[u8] = b"vibebuddy-vibegame-pair-v1";
const LINK_PROLOGUE: &[u8] = b"vibebuddy-vibegame-v1";
const STORE_FILE: &str = "vibegames.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How soon a lost VibeGame is tried again. Its app closes the port whenever it stops (another app in front, the
/// menu key, the lid), and the person expects the cards back as soon as it is in front again.
const RETRY: Duration = Duration::from_secs(2);
/// A VibeGame that talks to another computer is asked again after this.
const BUSY_RETRY: Duration = Duration::from_secs(15);
/// Lines queued for one VibeGame; a VibeGame that doesn't read loses lines, never holds the box up.
const QUEUE: usize = 64;

/// The events a VibeGame gets besides the heartbeat: the box's state events and the greeting.
const STATE_EVENTS: [&str; 7] = ["task.start", "agent.input_required", "task.done", "task.error", "agent.blocked", "agent.idle", "buddy.say"];
const HEARTBEAT: &str = "device.heartbeat";

/// One paired VibeGame, as kept in `vibegames.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Paired {
    key: String,
    name: String,
    /// The last address it answered at, `host:port`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

/// One VibeGame in `/v1/status`: found on the network, paired with this computer, or both.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VibeGameStatus {
    pub key: String,
    pub name: String,
    /// Paired with this computer.
    pub paired: bool,
    pub connected: bool,
    pub address: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    /// The code was wrong.
    Code,
    /// The VibeGame has four computers already.
    Full,
    /// The VibeGame isn't showing a code.
    Closed,
    /// Not found, or it didn't answer.
    Unreachable(String),
    /// It answered with something else, or proved another key.
    Refused(String),
}

impl PairError {
    /// What `POST /v1/vibegames/pair` answers.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Full => "full",
            Self::Closed => "closed",
            Self::Unreachable(_) | Self::Refused(_) => "unreachable",
        }
    }
}

/// What the daemon hears from its VibeGames.
#[derive(Debug)]
pub enum Inbound {
    /// A VibeGame connected, by key: it needs the screen as it is now.
    Connected(String),
    /// K2 or a picked card from a VibeGame.
    Event(Event),
}

enum Link {
    Connecting,
    Up { sender: mpsc::Sender<Vec<u8>>, address: String },
}

/// A VibeGame seen on the network: by mDNS, or by asking at the fixed address.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Found {
    key: String,
    name: String,
    addresses: Vec<String>,
}

pub struct Hub {
    identity: Option<Arc<Identity>>,
    store: Option<PathBuf>,
    paired: Mutex<Vec<Paired>>,
    fixed: Option<String>,
    discovery: Discovery,
    /// The key the VibeGame at the fixed address opened with.
    probed: Mutex<Option<String>>,
    links: Mutex<HashMap<String, Link>>,
    next_try: Mutex<HashMap<String, Instant>>,
    /// What [`Self::status`] last saw on the network, to tell the apps when it changes.
    last_found: Mutex<Vec<Found>>,
    box_linked: AtomicBool,
    changed: broadcast::Sender<()>,
    inbound: mpsc::UnboundedSender<Inbound>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Hub {
    /// `state_dir` holds `vibegames.json`; without an identity nothing is paired or connected.
    pub fn new(
        identity: Option<Arc<Identity>>,
        state_dir: Option<PathBuf>,
        fixed: Option<String>,
        changed: broadcast::Sender<()>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Inbound>) {
        let store = state_dir.map(|dir| dir.join(STORE_FILE));
        let paired = store
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let (inbound, receiver) = mpsc::unbounded_channel();
        let hub = Self {
            identity,
            store,
            paired: Mutex::new(paired),
            fixed,
            discovery: Discovery::new(SERVICE_TYPE),
            probed: Mutex::new(None),
            links: Mutex::new(HashMap::new()),
            next_try: Mutex::new(HashMap::new()),
            last_found: Mutex::new(Vec::new()),
            box_linked: AtomicBool::new(false),
            changed,
            inbound,
        };
        (Arc::new(hub), receiver)
    }

    /// Whether a box is linked to this computer: VibeGames are then `quiet`.
    pub fn set_box_linked(&self, linked: bool) {
        self.box_linked.store(linked, Ordering::Relaxed);
    }

    /// Sends a line the box got to every connected VibeGame, if it is one a VibeGame gets.
    pub fn send(&self, event: &Event) {
        let Some(frame) = viewer_frame(event, self.box_linked.load(Ordering::Relaxed)) else { return };
        for link in lock(&self.links).values() {
            if let Link::Up { sender, .. } = link {
                let _ = sender.try_send(frame.clone());
            }
        }
    }

    /// Sends a line to one VibeGame, as [`Self::send`] would.
    pub fn send_to(&self, key: &str, event: &Event) {
        let Some(frame) = viewer_frame(event, self.box_linked.load(Ordering::Relaxed)) else { return };
        if let Some(Link::Up { sender, .. }) = lock(&self.links).get(key) {
            let _ = sender.try_send(frame);
        }
    }

    fn notify(&self) {
        let _ = self.changed.send(());
    }

    fn paired_list(&self) -> Vec<Paired> {
        lock(&self.paired).clone()
    }

    fn save(&self, paired: &[Paired]) {
        let Some(path) = &self.store else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(paired) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(path, bytes) {
                    tracing::warn!(%error, "couldn't save the paired VibeGames");
                }
            }
            Err(error) => tracing::warn!(%error, "couldn't encode the paired VibeGames"),
        }
    }

    fn update_paired(&self, change: impl FnOnce(&mut Vec<Paired>)) {
        let snapshot = {
            let mut paired = lock(&self.paired);
            let before = paired.clone();
            change(&mut paired);
            if *paired == before {
                return;
            }
            paired.clone()
        };
        self.save(&snapshot);
        self.notify();
    }

    /// The VibeGames on the network now. At a fixed address, the one there once it has opened with its key.
    async fn found(&self) -> Vec<Found> {
        if let Some(address) = &self.fixed {
            if lock(&self.probed).is_none()
                && let Ok(key) = probe(address).await
            {
                *lock(&self.probed) = Some(key);
            }
            return lock(&self.probed)
                .iter()
                .map(|key| Found { key: key.clone(), name: default_name(key), addresses: vec![address.clone()] })
                .collect();
        }
        let mut found: Vec<Found> = Vec::new();
        for service in self.discovery.found().unwrap_or_default() {
            let Some(key) = service.key.filter(|key| lan::decode_key(key).is_some()) else { continue };
            let mut addresses: Vec<_> = service.addresses.clone();
            addresses.sort_by_key(|address| (!address.is_ipv4(), *address));
            let addresses = addresses.iter().map(ToString::to_string).collect();
            let name = service.name.filter(|name| !name.trim().is_empty()).unwrap_or_else(|| default_name(&key));
            match found.iter_mut().find(|other| other.key == key) {
                Some(other) => other.addresses.extend::<Vec<String>>(addresses),
                None => found.push(Found { key, name, addresses }),
            }
        }
        found.sort_by(|a, b| a.key.cmp(&b.key));
        found
    }

    /// Every VibeGame paired with this computer, then every other one found.
    pub async fn status(&self) -> Vec<VibeGameStatus> {
        let found = self.found().await;
        let links = lock(&self.links);
        let mut list: Vec<VibeGameStatus> = self
            .paired_list()
            .into_iter()
            .map(|paired| {
                let up = match links.get(&paired.key) {
                    Some(Link::Up { address, .. }) => Some(address.clone()),
                    _ => None,
                };
                VibeGameStatus {
                    connected: up.is_some(),
                    address: up.or(paired.address.clone()),
                    key: paired.key,
                    name: paired.name,
                    paired: true,
                }
            })
            .collect();
        for other in &found {
            if !list.iter().any(|known| known.key == other.key) {
                list.push(VibeGameStatus {
                    key: other.key.clone(),
                    name: other.name.clone(),
                    paired: false,
                    connected: false,
                    address: other.addresses.first().cloned(),
                });
            }
        }
        list
    }

    /// Keeps every paired VibeGame connected for the rest of the daemon's life, and tells the apps when what is
    /// found changes.
    pub async fn run(self: Arc<Self>) {
        let Some(identity) = self.identity.clone() else { return };
        loop {
            let found = self.found().await;
            let changed = {
                let mut last = lock(&self.last_found);
                let changed = *last != found;
                *last = found.clone();
                changed
            };
            if changed {
                self.notify();
            }
            for paired in self.paired_list() {
                if lock(&self.links).contains_key(&paired.key) {
                    continue;
                }
                if lock(&self.next_try).get(&paired.key).is_some_and(|next| *next > Instant::now()) {
                    continue;
                }
                let addresses = self.addresses(&paired, &found);
                if addresses.is_empty() {
                    continue;
                }
                lock(&self.links).insert(paired.key.clone(), Link::Connecting);
                tokio::spawn(Arc::clone(&self).keep(Arc::clone(&identity), paired, addresses));
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// Where to try a paired VibeGame: the fixed address alone, or where mDNS says it is, then where it last was.
    fn addresses(&self, paired: &Paired, found: &[Found]) -> Vec<String> {
        if let Some(fixed) = &self.fixed {
            return vec![fixed.clone()];
        }
        let mut addresses: Vec<String> = found.iter().filter(|other| other.key == paired.key).flat_map(|other| other.addresses.clone()).collect();
        if let Some(last) = &paired.address
            && !addresses.contains(last)
        {
            addresses.push(last.clone());
        }
        addresses
    }

    /// Connects to one VibeGame and serves it until the connection ends.
    async fn keep(self: Arc<Self>, identity: Arc<Identity>, paired: Paired, addresses: Vec<String>) {
        let key = paired.key.clone();
        let mut retry = RETRY;
        let mut connected = None;
        for address in addresses {
            match connect(&identity, &address, &key).await {
                Ok(link) => {
                    connected = Some((address, link));
                    break;
                }
                Err(LinkError::Busy { owner, .. }) => {
                    tracing::info!(%owner, "the VibeGame talks to another computer");
                    retry = BUSY_RETRY;
                    break;
                }
                Err(LinkError::Refused(code)) if code == "auth" => {
                    // The VibeGame forgot this computer ("Forget computers"): offer it for pairing again.
                    tracing::info!("the VibeGame doesn't know this computer any more; forgetting it here too");
                    self.update_paired(|list| list.retain(|other| other.key != key));
                    break;
                }
                Err(error) => tracing::debug!(?error, %address, "couldn't connect to the VibeGame"),
            }
        }
        let Some((address, (stream, hello))) = connected else {
            lock(&self.links).remove(&key);
            lock(&self.next_try).insert(key, Instant::now() + retry);
            return;
        };
        tracing::info!(build = %hello["build"].as_str().unwrap_or(""), "a VibeGame connected");
        let (sender, receiver) = mpsc::channel(QUEUE);
        lock(&self.links).insert(key.clone(), Link::Up { sender, address: address.clone() });
        let name = hello["name"].as_str().filter(|name| !name.trim().is_empty()).map(str::to_owned);
        self.update_paired(|list| {
            if let Some(entry) = list.iter_mut().find(|other| other.key == key) {
                entry.address = Some(address.clone());
                if let Some(name) = name {
                    entry.name = name;
                }
            }
        });
        self.notify();
        let _ = self.inbound.send(Inbound::Connected(key.clone()));
        self.serve(stream, receiver).await;
        tracing::info!("a VibeGame went away");
        lock(&self.links).remove(&key);
        lock(&self.next_try).insert(key, Instant::now() + RETRY);
        self.notify();
    }

    async fn serve(&self, stream: DuplexStream, mut outgoing: mpsc::Receiver<Vec<u8>>) {
        let (read, mut write) = tokio::io::split(stream);
        let mut lines = tokio::io::BufReader::new(read).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => self.received(&line),
                    _ => break,
                },
                frame = outgoing.recv() => match frame {
                    Some(frame) => if write.write_all(&frame).await.is_err() { break },
                    None => break,
                },
            }
        }
        let _ = write.shutdown().await;
    }

    /// A line from a VibeGame: K2 and a picked card go to the daemon; anything else is ignored.
    fn received(&self, line: &str) {
        let Ok(event) = serde_json::from_str::<Event>(line) else {
            tracing::debug!(length = line.len(), "a VibeGame sent a line that isn't an event");
            return;
        };
        let text = |name: &str| event.extra.get(name).and_then(Value::as_str);
        let k2 = event.event == "button" && text("button") == Some("K2") && text("action") == Some("press");
        let focus = event.event == "focus" && text("task").is_some();
        if k2 || focus {
            let _ = self.inbound.send(Inbound::Event(event));
        } else {
            tracing::debug!(event = %event.event, "ignoring a VibeGame event");
        }
    }

    /// Pairs with the VibeGame whose key is `key`, with the code it shows.
    pub async fn pair(&self, key: &str, code: &str) -> Result<(), PairError> {
        let identity = self.identity.clone().ok_or_else(|| PairError::Refused("this computer has no key".to_owned()))?;
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(PairError::Code);
        }
        let key_bytes = lan::decode_key(key).ok_or_else(|| PairError::Unreachable("not a VibeGame key".to_owned()))?;
        let found = self.found().await;
        let Some(found) = found.into_iter().find(|other| other.key == key) else {
            return Err(PairError::Unreachable("not found".to_owned()));
        };
        let mut last = PairError::Unreachable("no address".to_owned());
        for address in &found.addresses {
            let attempt = tokio::time::timeout(HANDSHAKE_TIMEOUT, pair_at(&identity, address, &key_bytes, code)).await;
            match attempt.unwrap_or_else(|_| Err(PairError::Unreachable("timed out".to_owned()))) {
                Ok(()) => {
                    tracing::info!("paired with a VibeGame");
                    lock(&self.next_try).remove(key);
                    let entry = Paired { key: key.to_owned(), name: found.name.clone(), address: Some(address.clone()) };
                    self.update_paired(|list| {
                        list.retain(|other| other.key != key);
                        list.push(entry);
                    });
                    return Ok(());
                }
                Err(PairError::Unreachable(error)) => last = PairError::Unreachable(error),
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// Forgets one VibeGame on this computer, telling it if it is connected. False if it wasn't paired.
    pub fn forget(&self, key: &str) -> bool {
        if !self.paired_list().iter().any(|other| other.key == key) {
            return false;
        }
        if let Some(Link::Up { sender, .. }) = lock(&self.links).remove(key) {
            let unpair = Event::named("vibegame.unpair");
            if let Ok(frame) = unpair.to_ndjson() {
                let _ = sender.try_send(frame);
            }
            // Dropping the sender ends the connection once the line is written.
        }
        lock(&self.next_try).remove(key);
        self.update_paired(|list| list.retain(|other| other.key != key));
        true
    }
}

/// What a VibeGame shows by when it has no `name`: its instance name.
fn default_name(key: &str) -> String {
    format!("VibeGame {}", key.get(..8).unwrap_or(key))
}

/// The line a VibeGame gets for something the box got, or None if a VibeGame doesn't get it. State events and the
/// greeting carry `quiet`, which is true while a box is linked: the box speaks, the VibeGame only shows.
pub fn viewer_frame(event: &Event, quiet: bool) -> Option<Vec<u8>> {
    if STATE_EVENTS.contains(&event.event.as_str()) {
        let mut event = event.clone();
        event.extra.insert("quiet".to_owned(), json!(quiet));
        return event.to_ndjson().ok();
    }
    (event.event == HEARTBEAT).then(|| event.to_ndjson().ok()).flatten()
}

async fn open(address: &str) -> Result<TcpStream, LinkError> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| LinkError::Unreachable("timed out".to_owned()))?
        .map_err(|error| LinkError::Unreachable(error.to_string()))?;
    let _ = stream.set_nodelay(true);
    lan::drop_when_unacknowledged(&stream, lan::GIVE_UP_AFTER);
    Ok(stream)
}

/// The key in a VibeGame's opening `link.noise`.
fn opener_key(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["event"] != "link.noise" {
        return None;
    }
    let key = value["vibegame"].as_str()?;
    lan::decode_key(key).map(|_| key.to_owned())
}

/// Asks what is at a fixed address: the key it opens with. It closes right after.
async fn probe(address: &str) -> Result<String, LinkError> {
    let mut stream = open(address).await?;
    let first = tokio::time::timeout(CONNECT_TIMEOUT, lan::read_line(&mut stream))
        .await
        .map_err(|_| LinkError::Unreachable("said nothing".to_owned()))??;
    opener_key(&first).ok_or_else(|| LinkError::Refused("not a VibeGame".to_owned()))
}

async fn write_line(stream: &mut TcpStream, line: &str) -> Result<(), LinkError> {
    stream.write_all(format!("{line}\n").as_bytes()).await.map_err(|error| LinkError::Unreachable(error.to_string()))
}

/// Connects to a paired VibeGame: `Noise_IK`, then its first sealed line is `vibegame.hello` or `link.busy`.
/// Returns the plain side and the hello.
async fn connect(identity: &Identity, address: &str, key: &str) -> Result<(DuplexStream, Value), LinkError> {
    let key_bytes = lan::decode_key(key).ok_or_else(|| LinkError::Refused("not a key".to_owned()))?;
    let mut stream = open(address).await?;
    let handshake = async {
        let first = lan::read_line(&mut stream).await?;
        if opener_key(&first).as_deref() != Some(key) {
            return Err(LinkError::Refused("a different VibeGame".to_owned()));
        }
        let (handshake, message) = noise_link::start_with(identity, &key_bytes, LINK_PROLOGUE, &[]).map_err(LinkError::Refused)?;
        write_line(&mut stream, &message).await?;
        let reply = lan::read_line(&mut stream).await?;
        if reply.starts_with('{') {
            let refused = lan::parse(&reply)?;
            return Err(lan::refusal(refused.get("event").and_then(Value::as_str), &refused));
        }
        let mut session = handshake.finish(&reply).map_err(LinkError::Refused)?;
        let sealed = lan::read_line(&mut stream).await?;
        let opened = session.open(&sealed).map_err(LinkError::Refused)?;
        let answer: Value = serde_json::from_slice(&opened).map_err(|_| LinkError::Refused("bad handshake line".to_owned()))?;
        match answer.get("event").and_then(Value::as_str) {
            Some("vibegame.hello") => Ok((session, answer)),
            event => Err(lan::refusal(event, &answer)),
        }
    };
    let (session, hello) = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake)
        .await
        .map_err(|_| LinkError::Unreachable("handshake timed out".to_owned()))??;
    Ok((noise_link::bridge(stream, session), hello))
}

/// Pairs at one address: `vibegame.pair_start` in the clear, `Noise_XX`, then the code inside the session.
async fn pair_at(identity: &Identity, address: &str, key: &[u8; 32], code: &str) -> Result<(), PairError> {
    let unreachable = |error: LinkError| match error {
        LinkError::Unreachable(error) => PairError::Unreachable(error),
        LinkError::Busy { reason, .. } => PairError::Refused(reason),
        LinkError::Refused(error) => PairError::Refused(error),
    };
    let mut stream = open(address).await.map_err(unreachable)?;
    let first = lan::read_line(&mut stream).await.map_err(unreachable)?;
    if opener_key(&first).as_deref() != Some(URL_SAFE_NO_PAD.encode(key).as_str()) {
        return Err(PairError::Refused("a different VibeGame".to_owned()));
    }
    write_line(&mut stream, r#"{"version":1,"event":"vibegame.pair_start"}"#).await.map_err(unreachable)?;
    let (pairing, message) = noise_link::start_pairing(identity, PAIR_PROLOGUE).map_err(PairError::Refused)?;
    write_line(&mut stream, &message).await.map_err(unreachable)?;
    let reply = lan::read_line(&mut stream).await.map_err(unreachable)?;
    if reply.starts_with('{') {
        let refused = lan::parse(&reply).map_err(unreachable)?;
        return Err(PairError::Refused(refused["code"].as_str().unwrap_or("unexpected reply").to_owned()));
    }
    let (mut session, last) = pairing.finish(&reply, key).map_err(PairError::Refused)?;
    write_line(&mut stream, &last).await.map_err(unreachable)?;
    let request = json!({"version": 1, "event": "vibegame.pair", "code": code, "name": identity.name}).to_string();
    let sealed = session.seal(request.as_bytes()).map_err(PairError::Refused)?;
    write_line(&mut stream, &sealed).await.map_err(unreachable)?;
    let answer = lan::read_line(&mut stream).await.map_err(unreachable)?;
    let answer = session.open(&answer).map_err(PairError::Refused)?;
    let answer: Value = serde_json::from_slice(&answer).map_err(|_| PairError::Refused("bad answer".to_owned()))?;
    match (answer["event"].as_str(), answer["reason"].as_str()) {
        (Some("vibegame.paired"), _) => Ok(()),
        (Some("vibegame.pair_error"), Some("code")) => Err(PairError::Code),
        (Some("vibegame.pair_error"), Some("full")) => Err(PairError::Full),
        (Some("vibegame.pair_error"), Some("closed")) => Err(PairError::Closed),
        _ => Err(PairError::Refused("unexpected answer".to_owned())),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    fn identity() -> Arc<Identity> {
        let mut suffix = [0u8; 4];
        getrandom::fill(&mut suffix).unwrap();
        let dir = std::env::temp_dir().join(format!("vibebuddy-vibegame-id-{}-{}", std::process::id(), u32::from_le_bytes(suffix)));
        Arc::new(Identity::load_or_create(&dir.join("identity"), "dragon's MacBook".to_owned()).unwrap())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut suffix = [0u8; 4];
        getrandom::fill(&mut suffix).unwrap();
        std::env::temp_dir().join(format!("vibebuddy-vibegame-{name}-{}-{}", std::process::id(), u32::from_le_bytes(suffix)))
    }

    /// A VibeGame played with snow directly, as docs/vibegame.md describes it.
    pub(crate) struct FakeVibeGame {
        pub address: String,
        pub key: String,
        /// Lines it got over the link, opened.
        pub received: mpsc::UnboundedReceiver<String>,
        /// Lines to send over the link.
        pub send: mpsc::UnboundedSender<String>,
        /// The computers it is paired with, as Noise saw their keys.
        pub computers: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    pub(crate) struct Behaviour {
        pub code: &'static str,
        pub full: bool,
        pub showing_code: bool,
        pub busy: bool,
    }

    impl Default for Behaviour {
        fn default() -> Self {
            Self { code: "123456", full: false, showing_code: true, busy: false }
        }
    }

    pub(crate) async fn fake_vibegame(behaviour: Behaviour) -> FakeVibeGame {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let signing = SigningKey::from_bytes(&[7; 32]);
        let key = URL_SAFE_NO_PAD.encode(signing.verifying_key().to_bytes());
        let secret = signing.to_scalar_bytes();
        let computers = Arc::new(Mutex::new(Vec::new()));
        let (received_tx, received) = mpsc::unbounded_channel();
        let (send, send_rx) = mpsc::unbounded_channel::<String>();
        let send_rx = Arc::new(tokio::sync::Mutex::new(send_rx));
        let shared = Arc::clone(&computers);
        let opener = json!({"version": 1, "event": "link.noise", "vibegame": key}).to_string();
        let behaviour = Arc::new(behaviour);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                let computers = Arc::clone(&shared);
                let received_tx = received_tx.clone();
                let send_rx = Arc::clone(&send_rx);
                let opener = opener.clone();
                let behaviour = Arc::clone(&behaviour);
                tokio::spawn(async move {
                    write.write_all(format!("{opener}\n").as_bytes()).await.unwrap();
                    let Ok(Some(first)) = lines.next_line().await else { return };
                    let mut buffer = vec![0u8; 65535];
                    if first.contains("vibegame.pair_start") {
                        let mut noise = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
                            .local_private_key(&secret)
                            .unwrap()
                            .prologue(PAIR_PROLOGUE)
                            .unwrap()
                            .build_responder()
                            .unwrap();
                        let one = lines.next_line().await.unwrap().unwrap();
                        noise.read_message(&URL_SAFE_NO_PAD.decode(one).unwrap(), &mut buffer).unwrap();
                        let length = noise.write_message(&[], &mut buffer).unwrap();
                        write.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(&buffer[..length])).as_bytes()).await.unwrap();
                        let three = lines.next_line().await.unwrap().unwrap();
                        noise.read_message(&URL_SAFE_NO_PAD.decode(three).unwrap(), &mut buffer).unwrap();
                        let computer = noise.get_remote_static().unwrap().to_vec();
                        let mut session = noise.into_transport_mode().unwrap();
                        let sealed = lines.next_line().await.unwrap().unwrap();
                        let length = session.read_message(&URL_SAFE_NO_PAD.decode(sealed).unwrap(), &mut buffer).unwrap();
                        let request: Value = serde_json::from_slice(&buffer[..length]).unwrap();
                        assert_eq!(request["name"], "dragon's MacBook");
                        let answer = if !behaviour.showing_code {
                            json!({"version":1,"event":"vibegame.pair_error","reason":"closed"})
                        } else if request["code"] != behaviour.code {
                            json!({"version":1,"event":"vibegame.pair_error","reason":"code"})
                        } else if behaviour.full {
                            json!({"version":1,"event":"vibegame.pair_error","reason":"full"})
                        } else {
                            computers.lock().unwrap().push(computer);
                            json!({"version":1,"event":"vibegame.paired"})
                        };
                        let length = session.write_message(answer.to_string().as_bytes(), &mut buffer).unwrap();
                        write.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(&buffer[..length])).as_bytes()).await.unwrap();
                        return;
                    }
                    let mut noise = snow::Builder::new("Noise_IK_25519_ChaChaPoly_SHA256".parse().unwrap())
                        .local_private_key(&secret)
                        .unwrap()
                        .prologue(LINK_PROLOGUE)
                        .unwrap()
                        .build_responder()
                        .unwrap();
                    noise.read_message(&URL_SAFE_NO_PAD.decode(&first).unwrap(), &mut buffer).unwrap();
                    let computer = noise.get_remote_static().unwrap().to_vec();
                    if !computers.lock().unwrap().contains(&computer) {
                        write.write_all(b"{\"version\":1,\"event\":\"link.error\",\"code\":\"auth\"}\n").await.unwrap();
                        return;
                    }
                    let length = noise.write_message(&[], &mut buffer).unwrap();
                    write.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(&buffer[..length])).as_bytes()).await.unwrap();
                    let mut session = noise.into_transport_mode().unwrap();
                    let first_line = if behaviour.busy {
                        json!({"version":1,"event":"link.busy","reason":"linked","owner":"Other Mac"})
                    } else {
                        json!({"version":1,"event":"vibegame.hello","build":"test","name":"Desk VibeGame","screen":[720,480]})
                    };
                    let length = session.write_message(first_line.to_string().as_bytes(), &mut buffer).unwrap();
                    write.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(&buffer[..length])).as_bytes()).await.unwrap();
                    if behaviour.busy {
                        return;
                    }
                    let mut send_rx = send_rx.lock().await;
                    loop {
                        tokio::select! {
                            line = lines.next_line() => {
                                let Ok(Some(line)) = line else { return };
                                let length = session.read_message(&URL_SAFE_NO_PAD.decode(line).unwrap(), &mut buffer).unwrap();
                                let _ = received_tx.send(String::from_utf8(buffer[..length].to_vec()).unwrap());
                            }
                            outgoing = send_rx.recv() => {
                                let Some(outgoing) = outgoing else { return };
                                let length = session.write_message(outgoing.as_bytes(), &mut buffer).unwrap();
                                if write.write_all(format!("{}\n", URL_SAFE_NO_PAD.encode(&buffer[..length])).as_bytes()).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        FakeVibeGame { address, key, received, send, computers }
    }

    pub(crate) fn hub_for(vibegame: &FakeVibeGame, dir: Option<PathBuf>) -> (Arc<Hub>, mpsc::UnboundedReceiver<Inbound>) {
        let (changed, _) = broadcast::channel(16);
        Hub::new(Some(identity()), dir, Some(vibegame.address.clone()), changed)
    }

    /// The next opened line the VibeGame got, as JSON.
    pub(crate) async fn next_line(vibegame: &mut FakeVibeGame) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), vibegame.received.recv()).await.expect("a line in time").expect("the link is up");
        serde_json::from_str(&line).unwrap()
    }

    pub(crate) async fn wait_connected(hub: &Hub, key: &str) {
        for _ in 0..200 {
            if hub.status().await.iter().any(|vibegame| vibegame.key == key && vibegame.connected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the VibeGame never connected");
    }

    #[tokio::test]
    async fn the_right_code_pairs_and_the_link_comes_up() {
        let mut vibegame = fake_vibegame(Behaviour::default()).await;
        let dir = temp_dir("pair");
        let (hub, mut inbound) = hub_for(&vibegame, Some(dir.clone()));
        let found = hub.status().await;
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].key.as_str(), found[0].paired), (vibegame.key.as_str(), false));

        hub.pair(&vibegame.key, "123456").await.unwrap();
        assert_eq!(vibegame.computers.lock().unwrap().len(), 1);
        let stored: Vec<Paired> = serde_json::from_slice(&std::fs::read(dir.join(STORE_FILE)).unwrap()).unwrap();
        assert_eq!(stored[0].key, vibegame.key);
        assert_eq!(stored[0].address.as_deref(), Some(vibegame.address.as_str()));

        tokio::spawn(Arc::clone(&hub).run());
        wait_connected(&hub, &vibegame.key).await;
        assert!(matches!(inbound.recv().await, Some(Inbound::Connected(key)) if key == vibegame.key));
        let status = hub.status().await;
        assert_eq!(status[0].name, "Desk VibeGame", "the name from vibegame.hello");

        let mut done = Event::named("task.done");
        done.extra.insert("occasion".to_owned(), json!("first_done_today"));
        hub.send(&done);
        let line = next_line(&mut vibegame).await;
        assert_eq!((line["event"].as_str(), line["quiet"].as_bool()), (Some("task.done"), Some(false)), "no box: the VibeGame speaks");
        assert_eq!(line["occasion"], "first_done_today");

        hub.set_box_linked(true);
        hub.send(&done);
        assert_eq!(next_line(&mut vibegame).await["quiet"], true, "a box is linked: the VibeGame only shows");

        vibegame.send.send(r#"{"version":1,"event":"button","button":"K2","action":"press"}"#.to_owned()).unwrap();
        vibegame.send.send(r#"{"version":1,"event":"voice.start"}"#.to_owned()).unwrap();
        vibegame.send.send(r#"{"version":1,"event":"focus","task":"codex:1"}"#.to_owned()).unwrap();
        let k2 = tokio::time::timeout(Duration::from_secs(5), inbound.recv()).await.unwrap();
        assert!(matches!(k2, Some(Inbound::Event(event)) if event.event == "button"));
        let focus = tokio::time::timeout(Duration::from_secs(5), inbound.recv()).await.unwrap();
        assert!(matches!(focus, Some(Inbound::Event(event)) if event.event == "focus"), "voice.* from a VibeGame is ignored");

        assert!(hub.forget(&vibegame.key));
        assert_eq!(next_line(&mut vibegame).await["event"], "vibegame.unpair");
        let status = hub.status().await;
        assert!(!status[0].paired && !status[0].connected);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_wrong_code_does_not_pair() {
        let vibegame = fake_vibegame(Behaviour::default()).await;
        let (hub, _) = hub_for(&vibegame, None);
        assert_eq!(hub.pair(&vibegame.key, "654321").await, Err(PairError::Code));
        assert_eq!(hub.pair(&vibegame.key, "12345").await, Err(PairError::Code), "not six digits: never sent");
        assert!(vibegame.computers.lock().unwrap().is_empty());
        assert!(hub.status().await.iter().all(|vibegame| !vibegame.paired));
    }

    #[tokio::test]
    async fn a_full_or_closed_vibegame_does_not_pair() {
        let full = fake_vibegame(Behaviour { full: true, ..Behaviour::default() }).await;
        let (hub, _) = hub_for(&full, None);
        assert_eq!(hub.pair(&full.key, "123456").await, Err(PairError::Full));
        assert_eq!(PairError::Full.code(), "full");
        let closed = fake_vibegame(Behaviour { showing_code: false, ..Behaviour::default() }).await;
        let (hub, _) = hub_for(&closed, None);
        assert_eq!(hub.pair(&closed.key, "123456").await, Err(PairError::Closed));
    }

    #[tokio::test]
    async fn an_unknown_vibegame_is_unreachable() {
        let (changed, _) = broadcast::channel(16);
        let (hub, _) = Hub::new(Some(identity()), None, Some("127.0.0.1:9".to_owned()), changed);
        let error = hub.pair(&URL_SAFE_NO_PAD.encode([5u8; 32]), "123456").await.unwrap_err();
        assert_eq!(error.code(), "unreachable");
    }

    #[tokio::test]
    async fn a_vibegame_that_forgot_this_computer_is_forgotten_here() {
        let vibegame = fake_vibegame(Behaviour::default()).await;
        let (hub, _) = hub_for(&vibegame, None);
        hub.pair(&vibegame.key, "123456").await.unwrap();
        vibegame.computers.lock().unwrap().clear();
        tokio::spawn(Arc::clone(&hub).run());
        for _ in 0..100 {
            if hub.status().await.iter().all(|vibegame| !vibegame.paired) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("still paired after link.error auth");
    }

    #[tokio::test]
    async fn a_busy_vibegame_is_not_connected() {
        let vibegame = fake_vibegame(Behaviour { busy: true, ..Behaviour::default() }).await;
        let (hub, _) = hub_for(&vibegame, None);
        hub.pair(&vibegame.key, "123456").await.unwrap();
        let result = connect(hub.identity.as_ref().unwrap(), &vibegame.address, &vibegame.key).await;
        assert!(matches!(result, Err(LinkError::Busy { reason, owner }) if reason == "linked" && owner == "Other Mac"));
    }

    #[test]
    fn a_vibegame_gets_the_snapshot_but_nothing_that_looks_after_a_box() {
        let mut idle = Event::named("agent.idle");
        idle.extra.insert("suppress_audio".to_owned(), json!(true));
        let frame: Value = serde_json::from_slice(&viewer_frame(&idle, false).unwrap()).unwrap();
        assert_eq!((frame["quiet"].as_bool(), frame["suppress_audio"].as_bool()), (Some(false), Some(true)));
        let say: Value = serde_json::from_slice(&viewer_frame(&Event::named("buddy.say"), true).unwrap()).unwrap();
        assert_eq!(say["quiet"], true);
        let heartbeat: Value = serde_json::from_slice(&viewer_frame(&Event::named("device.heartbeat"), true).unwrap()).unwrap();
        assert!(heartbeat.get("quiet").is_none(), "only state events and buddy.say carry quiet");
        for name in ["device.hello", "device.pair", "device.wifi.set", "device.relay.set", "xfer.begin", "voice.reply", "voice.offer", "device.screens"] {
            assert!(viewer_frame(&Event::named(name), false).is_none(), "{name} isn't sent to a VibeGame");
        }
    }
}
