//! The computer's side of the local network link (ADR-0012, docs/protocol.md "Local network link"):
//! where the box is (remembered from USB, or found by mDNS when it moved), and the handshake in which both ends prove their keys before any VibeBuddy
//! Protocol line goes over the connection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::identity::Identity;

/// The port a box listens on.
pub const PORT: u16 = 7340;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HANDSHAKE_LINE: usize = 1024;

/// What the daemon needs to reach the box without the cable.
pub struct LanConfig {
    pub identity: Arc<Identity>,
    /// Where the box's key and last address are remembered, learned over USB.
    pub state_dir: PathBuf,
    /// `VIBEBUDDY_LAN_ADDR`: a fixed host:port, for the simulator and for networks where the box's address is known.
    pub fixed_address: Option<String>,
    /// What boxes advertise over mDNS, listened for from the first time the remembered address doesn't answer.
    pub discovery: Discovery,
}

/// One box to try: where it is and which key it must prove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub address: String,
    pub box_key: [u8; 32],
}

impl LanConfig {
    /// The key of the box this computer paired with, learned over USB.
    pub fn box_key(&self) -> Option<[u8; 32]> {
        read_key(&box_key_file(&self.state_dir))
    }

    /// The remembered box, if this computer has been paired with one and knows where it is.
    pub fn target(&self) -> Option<Target> {
        let box_key = self.box_key()?;
        let address = match &self.fixed_address {
            Some(address) => address.clone(),
            None => {
                let host = std::fs::read_to_string(box_address_file(&self.state_dir)).ok()?;
                format!("{}:{PORT}", host.trim())
            }
        };
        Some(Target { address, box_key })
    }
}

impl LanConfig {
    /// Where mDNS says the box with this key is now; nothing with a fixed address.
    pub fn advertised(&self, box_key: &[u8; 32]) -> Vec<String> {
        if self.fixed_address.is_some() {
            return Vec::new();
        }
        self.discovery.found().map_or_else(Vec::new, |found| pick(&found, box_key))
    }
}

impl LanConfig {
    /// [`advertised`](Self::advertised), but a browse that only starts now has heard nothing yet: it gets a moment
    /// for the box to answer its first query.
    async fn find(&self, box_key: &[u8; 32]) -> Vec<String> {
        let fresh = self.discovery.browser.get().is_none();
        let mut found = self.advertised(box_key);
        for _ in 0..20 {
            if !fresh || !found.is_empty() || self.fixed_address.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            found = self.advertised(box_key);
        }
        found
    }
}

/// The DNS-SD service type the box advertises once it is on Wi-Fi (docs/protocol.md).
pub const SERVICE_TYPE: &str = "_vibebuddy._tcp.local.";

/// One service instance an mDNS browse resolved: where it listens and the box key in its TXT record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertised {
    pub addresses: Vec<SocketAddr>,
    pub key: Option<String>,
}

/// The addresses of the instances whose TXT key is this box's, IPv4 first. Only finds the box: the handshake
/// still checks its key.
pub fn pick(found: &[Advertised], box_key: &[u8; 32]) -> Vec<String> {
    let mut addresses: Vec<SocketAddr> = found
        .iter()
        .filter(|service| service.key.as_deref().and_then(decode::<32>) == Some(*box_key))
        .flat_map(|service| service.addresses.iter().copied())
        .collect();
    addresses.sort_by_key(|address| (!address.is_ipv4(), *address));
    addresses.dedup();
    addresses.iter().map(SocketAddr::to_string).collect()
}

/// A browse for [`SERVICE_TYPE`] that runs for the rest of the daemon's life once started, keeping what it
/// resolved so a lookup is immediate. It listens for the box's announcements rather than asking every time.
#[derive(Default)]
pub struct Discovery {
    browser: OnceLock<Option<Browser>>,
}

struct Browser {
    /// Keeps the browse running; None only in tests that fill `found` themselves.
    _daemon: Option<mdns_sd::ServiceDaemon>,
    found: Arc<Mutex<HashMap<String, Advertised>>>,
}

impl Discovery {
    /// What has been resolved so far, starting the browse on first use; None if mDNS couldn't start.
    fn found(&self) -> Option<Vec<Advertised>> {
        let browser = self.browser.get_or_init(|| match Browser::start() {
            Ok(browser) => Some(browser),
            Err(error) => {
                tracing::warn!(%error, "couldn't browse for the box over mDNS");
                None
            }
        });
        let found = browser.as_ref()?.found.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(found.values().cloned().collect())
    }
}

impl Browser {
    fn start() -> Result<Self, mdns_sd::Error> {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        let events = daemon.browse(SERVICE_TYPE)?;
        let found = Arc::new(Mutex::new(HashMap::new()));
        let shared = Arc::clone(&found);
        std::thread::Builder::new()
            .name("mdns-browse".to_owned())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    let mut found = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    match event {
                        mdns_sd::ServiceEvent::ServiceResolved(service) => {
                            let addresses = service.addresses.iter().map(|ip| SocketAddr::new(ip.to_ip_addr(), service.port)).collect();
                            let key = service.txt_properties.get_property_val_str("key").map(str::to_owned);
                            tracing::debug!(name = %service.fullname, ?addresses, "a box advertised itself over mDNS");
                            found.insert(service.fullname.clone(), Advertised { addresses, key });
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, name) => {
                            found.remove(&name);
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|error| mdns_sd::Error::Msg(error.to_string()))?;
        Ok(Self { _daemon: Some(daemon), found })
    }
}

pub fn box_key_file(state_dir: &Path) -> PathBuf {
    state_dir.join("box-key")
}

pub fn box_address_file(state_dir: &Path) -> PathBuf {
    state_dir.join("box-address")
}

/// Writes what was learned about the box over USB: its key, or the address it got on Wi-Fi.
pub fn remember(path: &Path, value: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{value}\n"));
}

fn read_key(path: &Path) -> Option<[u8; 32]> {
    let text = std::fs::read_to_string(path).ok()?;
    URL_SAFE_NO_PAD.decode(text.trim()).ok()?.try_into().ok()
}

/// What a side signs: bound to its role, the box and the nonce the other side chose.
pub fn signed_message(role: &str, box_key: &[u8; 32], nonce: &[u8; 32]) -> Vec<u8> {
    format!("vibebuddy-lan-v1\n{role}\n{}\n{}", URL_SAFE_NO_PAD.encode(box_key), URL_SAFE_NO_PAD.encode(nonce)).into_bytes()
}

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Couldn't reach it, or the connection broke.
    Unreachable(String),
    /// The box is linked elsewhere: `usb` or `linked` (docs/protocol.md), and to whom when it says.
    Busy { reason: String, owner: String },
    /// It refused us (`auth`, `bad_message`, `timeout`), or it isn't the box we paired with.
    Refused(String),
}

/// Data the box hasn't acknowledged for this long means it is gone. The box sends nothing on its own, so a link
/// whose other end vanished without a word (this computer moved to another network, the box lost power) would
/// otherwise look alive for minutes while the kernel retransmits. The daemon's heartbeat every 5 s always leaves
/// something to acknowledge.
const GIVE_UP_AFTER: Duration = Duration::from_secs(20);

/// Has the kernel drop the connection once sent data stays unacknowledged for `after`; the read then fails and the
/// daemon tries the next way to the box.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn drop_when_unacknowledged(stream: &TcpStream, after: Duration) {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "macos")]
    let (option, value) = {
        // TCP_RXT_CONNDROPTIME from <netinet/tcp.h>, in seconds; libc doesn't export it.
        const TCP_RXT_CONNDROPTIME: libc::c_int = 0x80;
        (TCP_RXT_CONNDROPTIME, after.as_secs() as libc::c_int)
    };
    #[cfg(target_os = "linux")]
    let (option, value) = (libc::TCP_USER_TIMEOUT, after.as_millis() as libc::c_int);
    // SAFETY: a valid socket descriptor and a c_int option value of the right size.
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::IPPROTO_TCP,
            option,
            (&raw const value).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result != 0 {
        tracing::debug!(error = %std::io::Error::last_os_error(), "couldn't set the connection's give-up time");
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn drop_when_unacknowledged(_stream: &TcpStream, _after: Duration) {}

/// Connects and runs the handshake; on success the stream carries the VibeBuddy Protocol.
pub async fn connect(target: &Target, identity: &Identity, takeover: bool) -> Result<TcpStream, LinkError> {
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&target.address))
        .await
        .map_err(|_| LinkError::Unreachable("timed out".to_owned()))?
        .map_err(|error| LinkError::Unreachable(error.to_string()))?;
    let _ = stream.set_nodelay(true);
    drop_when_unacknowledged(&stream, GIVE_UP_AFTER);
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, target, identity, takeover))
        .await
        .map_err(|_| LinkError::Unreachable("handshake timed out".to_owned()))??;
    Ok(stream)
}

/// Links to the paired box over the local network: at the remembered address, or, when nothing answers there,
/// wherever mDNS says it is now. A new address is remembered once the handshake proves the box's key. None when
/// this computer isn't paired or knows nowhere to try; otherwise the last place tried and how it went.
pub async fn link(lan: &LanConfig, takeover: bool) -> Option<(Target, Result<TcpStream, LinkError>)> {
    let box_key = lan.box_key()?;
    let remembered = lan.target();
    let mut last = None;
    if let Some(target) = &remembered {
        let result = connect(target, &lan.identity, takeover).await;
        if !matches!(result, Err(LinkError::Unreachable(_))) {
            return Some((target.clone(), result));
        }
        last = Some((target.clone(), result));
    }
    for address in lan.find(&box_key).await {
        if remembered.as_ref().is_some_and(|target| target.address == address) {
            continue;
        }
        let target = Target { address, box_key };
        let result = connect(&target, &lan.identity, takeover).await;
        if result.is_ok()
            && let Ok(found) = target.address.parse::<SocketAddr>()
        {
            tracing::info!(address = %target.address, "found the box over mDNS");
            remember(&box_address_file(&lan.state_dir), &found.ip().to_string());
        }
        if !matches!(result, Err(LinkError::Unreachable(_))) {
            return Some((target, result));
        }
        last = Some((target, result));
    }
    last
}

/// Whether the paired box answers on the local network, at the remembered address or one mDNS found: while
/// linked through the Relay, a reason to move over.
pub async fn reachable(lan: &LanConfig) -> bool {
    let Some(box_key) = lan.box_key() else { return false };
    let remembered = lan.target().map(|target| target.address);
    let found = lan.advertised(&box_key).into_iter().filter(|address| remembered.as_ref() != Some(address));
    for address in remembered.clone().into_iter().chain(found) {
        if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(&address)).await {
            return true;
        }
    }
    false
}

pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    target: &Target,
    identity: &Identity,
    takeover: bool,
) -> Result<(), LinkError> {
    let challenge = read_message(stream).await?;
    let text = |value: &Value, name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    if text(&challenge, "event").as_deref() != Some("link.challenge") {
        return Err(LinkError::Refused("no challenge".to_owned()));
    }
    // Another box answering at the remembered address is never driven.
    let box_key = text(&challenge, "box").and_then(|key| decode::<32>(&key));
    if box_key != Some(target.box_key) {
        return Err(LinkError::Refused("a different box".to_owned()));
    }
    let nonce = text(&challenge, "nonce").and_then(|nonce| decode::<32>(&nonce)).ok_or_else(|| LinkError::Refused("bad challenge".to_owned()))?;
    let mut ours = [0u8; 32];
    getrandom::fill(&mut ours).map_err(|error| LinkError::Unreachable(error.to_string()))?;
    let hello = serde_json::json!({
        "version": 1,
        "event": "link.hello",
        "key": identity.public_key(),
        "sig": URL_SAFE_NO_PAD.encode(identity.sign(&signed_message("computer", &target.box_key, &nonce))),
        "nonce": URL_SAFE_NO_PAD.encode(ours),
        "takeover": takeover,
    });
    let mut line = hello.to_string().into_bytes();
    line.push(b'\n');
    stream.write_all(&line).await.map_err(|error| LinkError::Unreachable(error.to_string()))?;

    let answer = read_message(stream).await?;
    match text(&answer, "event").as_deref() {
        Some("link.welcome") => {
            let signature = text(&answer, "sig").and_then(|sig| decode::<64>(&sig));
            let proven = signature.is_some_and(|signature| verify(&target.box_key, &signed_message("box", &target.box_key, &ours), &signature));
            if proven { Ok(()) } else { Err(LinkError::Refused("the box didn't prove its key".to_owned())) }
        }
        Some("link.busy") => Err(LinkError::Busy { reason: text(&answer, "reason").unwrap_or_default(), owner: text(&answer, "owner").unwrap_or_default() }),
        _ => Err(LinkError::Refused(text(&answer, "code").unwrap_or_else(|| "unexpected reply".to_owned()))),
    }
}

/// Reads one line, byte by byte so nothing after it is taken from the stream.
async fn read_message<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Value, LinkError> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        match stream.read(&mut byte).await {
            Ok(0) => return Err(LinkError::Unreachable("closed during the handshake".to_owned())),
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) if line.len() < MAX_HANDSHAKE_LINE => line.push(byte[0]),
            Ok(_) => return Err(LinkError::Refused("handshake line too long".to_owned())),
            Err(error) => return Err(LinkError::Unreachable(error.to_string())),
        }
    }
    serde_json::from_slice(&line).map_err(|_| LinkError::Refused("bad handshake line".to_owned()))
}

fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    URL_SAFE_NO_PAD.decode(text).ok()?.try_into().ok()
}

fn verify(key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    ed25519_dalek::VerifyingKey::from_bytes(key)
        .is_ok_and(|key| key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(signature)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_link_gives_up_on_data_the_box_never_acknowledges() {
        use std::os::fd::AsRawFd;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        drop_when_unacknowledged(&stream, GIVE_UP_AFTER);
        let mut value: libc::c_int = 0;
        let mut length = size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: a valid socket and a c_int-sized buffer.
        let result = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::IPPROTO_TCP, 0x80, (&raw mut value).cast(), &mut length) };
        assert_eq!((result, value), (0, 20));
    }
    use ed25519_dalek::{Signer, SigningKey};
    use tokio::io::AsyncBufReadExt;

    fn identity() -> Identity {
        let dir = std::env::temp_dir().join(format!("vibebuddy-lan-test-{}-{}", std::process::id(), rand_suffix()));
        Identity::load_or_create(&dir.join("identity"), "test".to_owned()).unwrap()
    }

    fn rand_suffix() -> u32 {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).unwrap();
        u32::from_le_bytes(bytes)
    }

    /// Plays the box's side of the handshake on the other end of a duplex pipe.
    async fn fake_box<S: AsyncRead + AsyncWrite + Send + 'static>(stream: S, box_key: SigningKey, reply: &'static str) {
        let (read, mut write) = tokio::io::split(stream);
        let mut lines = tokio::io::BufReader::new(read).lines();
        let public = box_key.verifying_key().to_bytes();
        let challenge = serde_json::json!({"version":1,"event":"link.challenge","box":URL_SAFE_NO_PAD.encode(public),"nonce":URL_SAFE_NO_PAD.encode([4u8; 32])});
        write.write_all(format!("{challenge}\n").as_bytes()).await.unwrap();
        let Ok(Some(hello)) = lines.next_line().await else { return };
        let hello: Value = serde_json::from_str(&hello).unwrap();
        let theirs = decode::<32>(hello["nonce"].as_str().unwrap()).unwrap();
        let answer = match reply {
            "welcome" => {
                let sig = box_key.sign(&signed_message("box", &public, &theirs)).to_bytes();
                serde_json::json!({"version":1,"event":"link.welcome","sig":URL_SAFE_NO_PAD.encode(sig)})
            }
            "forged" => serde_json::json!({"version":1,"event":"link.welcome","sig":URL_SAFE_NO_PAD.encode([0u8; 64])}),
            _ => serde_json::json!({"version":1,"event":"link.busy","reason":"usb","owner":""}),
        };
        write.write_all(format!("{answer}\n").as_bytes()).await.unwrap();
    }

    async fn run(reply: &'static str, expected_box: Option<[u8; 32]>) -> Result<(), LinkError> {
        let box_key = SigningKey::from_bytes(&[8; 32]);
        let target = Target { address: String::new(), box_key: expected_box.unwrap_or(box_key.verifying_key().to_bytes()) };
        let (mut ours, theirs) = tokio::io::duplex(4096);
        tokio::spawn(fake_box(theirs, box_key, reply));
        handshake(&mut ours, &target, &identity(), false).await
    }

    #[tokio::test]
    async fn the_box_proves_its_key() {
        assert_eq!(run("welcome", None).await, Ok(()));
    }

    #[tokio::test]
    async fn a_welcome_without_the_box_key_is_refused() {
        assert!(matches!(run("forged", None).await, Err(LinkError::Refused(_))));
    }

    #[tokio::test]
    async fn another_box_at_the_address_is_never_driven() {
        assert_eq!(run("welcome", Some([1; 32])).await, Err(LinkError::Refused("a different box".to_owned())));
    }

    #[tokio::test]
    async fn busy_says_why() {
        assert_eq!(run("busy", None).await, Err(LinkError::Busy { reason: "usb".to_owned(), owner: String::new() }));
    }

    #[test]
    fn the_target_needs_a_remembered_key() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-lan-target-{}-{}", std::process::id(), rand_suffix()));
        let config = LanConfig { identity: Arc::new(identity()), state_dir: dir.clone(), fixed_address: Some("127.0.0.1:7341".to_owned()), discovery: Discovery::default() };
        assert_eq!(config.target(), None);
        remember(&box_key_file(&dir), &URL_SAFE_NO_PAD.encode([3u8; 32]));
        assert_eq!(config.target(), Some(Target { address: "127.0.0.1:7341".to_owned(), box_key: [3; 32] }));
        let config = LanConfig { fixed_address: None, ..config };
        assert_eq!(config.target(), None, "no address learned yet");
        remember(&box_address_file(&dir), "192.168.1.23");
        assert_eq!(config.target().map(|target| target.address), Some(format!("192.168.1.23:{PORT}")));
        std::fs::remove_dir_all(dir).ok();
    }

    fn advertised(addresses: &[&str], key: Option<[u8; 32]>) -> Advertised {
        Advertised { addresses: addresses.iter().map(|address| address.parse().unwrap()).collect(), key: key.map(|key| URL_SAFE_NO_PAD.encode(key)) }
    }

    #[test]
    fn mdns_picks_the_box_whose_key_matches_among_several() {
        let found = [
            advertised(&["192.168.1.40:7340"], Some([1; 32])),
            advertised(&["[fe80::2]:7340", "192.168.1.41:7340"], Some([2; 32])),
            advertised(&["192.168.1.42:7340"], None),
            advertised(&["192.168.1.43:7340"], Some([3; 32])),
        ];
        assert_eq!(pick(&found, &[2; 32]), ["192.168.1.41:7340", "[fe80::2]:7340"], "IPv4 first");
        assert_eq!(pick(&found, &[1; 32]), ["192.168.1.40:7340"]);
    }

    #[test]
    fn mdns_without_a_matching_key_finds_nothing() {
        let found = [
            advertised(&["192.168.1.40:7340"], Some([1; 32])),
            advertised(&["192.168.1.42:7340"], None),
            Advertised { addresses: vec!["192.168.1.44:7340".parse().unwrap()], key: Some("not a key".to_owned()) },
        ];
        assert!(pick(&found, &[9; 32]).is_empty());
        assert!(pick(&[], &[9; 32]).is_empty());
    }

    #[tokio::test]
    async fn an_unmatched_key_falls_back_to_the_remembered_address() {
        // Nothing listens at the remembered address and mDNS found no box with this key: the link reports the
        // remembered address as unreachable, as before discovery, and the daemon moves on to the Relay.
        let dir = std::env::temp_dir().join(format!("vibebuddy-lan-fallback-{}-{}", std::process::id(), rand_suffix()));
        remember(&box_key_file(&dir), &URL_SAFE_NO_PAD.encode([3u8; 32]));
        remember(&box_address_file(&dir), "127.0.0.2");
        let config = LanConfig { identity: Arc::new(identity()), state_dir: dir.clone(), fixed_address: None, discovery: Discovery::default() };
        let found = Arc::new(Mutex::new(HashMap::from([("other".to_owned(), advertised(&["127.0.0.1:1"], Some([4; 32])))])));
        let _ = config.discovery.browser.set(Some(Browser { _daemon: None, found }));
        assert!(config.advertised(&[3; 32]).is_empty());
        let (target, result) = link(&config, false).await.unwrap();
        assert_eq!(target.address, format!("127.0.0.2:{PORT}"));
        assert!(matches!(result, Err(LinkError::Unreachable(_))));
        assert_eq!(std::fs::read_to_string(box_address_file(&dir)).unwrap().trim(), "127.0.0.2", "nothing new remembered");
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn mdns_finds_the_box_after_its_address_changed() {
        let box_key = SigningKey::from_bytes(&[8; 32]);
        let public = box_key.verifying_key().to_bytes();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let moved = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            fake_box(stream, box_key, "welcome").await;
        });
        let dir = std::env::temp_dir().join(format!("vibebuddy-lan-moved-{}-{}", std::process::id(), rand_suffix()));
        remember(&box_key_file(&dir), &URL_SAFE_NO_PAD.encode(public));
        remember(&box_address_file(&dir), "127.0.0.2");
        let config = LanConfig { identity: Arc::new(identity()), state_dir: dir.clone(), fixed_address: None, discovery: Discovery::default() };
        let found = HashMap::from([
            ("another".to_owned(), advertised(&["127.0.0.1:1"], Some([4; 32]))),
            ("ours".to_owned(), advertised(&[&moved.to_string()], Some(public))),
        ]);
        let _ = config.discovery.browser.set(Some(Browser { _daemon: None, found: Arc::new(Mutex::new(found)) }));
        let (target, result) = link(&config, false).await.unwrap();
        assert_eq!(target.address, moved.to_string());
        assert!(result.is_ok());
        assert_eq!(std::fs::read_to_string(box_address_file(&dir)).unwrap().trim(), "127.0.0.1", "the new address is remembered");
        std::fs::remove_dir_all(dir).ok();
    }
}
