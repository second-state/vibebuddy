//! The computer's side of the local network link (ADR-0012, docs/protocol.md "Local network link"):
//! where the box is, and the handshake in which both ends prove their keys before any VibeBuddy
//! Protocol line goes over the connection.

use std::path::{Path, PathBuf};
use std::sync::Arc;
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
}

/// One box to try: where it is and which key it must prove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub address: String,
    pub box_key: [u8; 32],
}

impl LanConfig {
    /// The remembered box, if this computer has been paired with one and knows where it is.
    pub fn target(&self) -> Option<Target> {
        let box_key = read_key(&box_key_file(&self.state_dir))?;
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
    /// The box is linked elsewhere: `usb` or `linked` (docs/protocol.md).
    Busy(String),
    /// It refused us (`auth`, `bad_message`, `timeout`), or it isn't the box we paired with.
    Refused(String),
}

/// Connects and runs the handshake; on success the stream carries the VibeBuddy Protocol.
pub async fn connect(target: &Target, identity: &Identity, takeover: bool) -> Result<TcpStream, LinkError> {
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&target.address))
        .await
        .map_err(|_| LinkError::Unreachable("timed out".to_owned()))?
        .map_err(|error| LinkError::Unreachable(error.to_string()))?;
    let _ = stream.set_nodelay(true);
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, target, identity, takeover))
        .await
        .map_err(|_| LinkError::Unreachable("handshake timed out".to_owned()))??;
    Ok(stream)
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
        Some("link.busy") => Err(LinkError::Busy(text(&answer, "reason").unwrap_or_default())),
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
    async fn fake_box(stream: tokio::io::DuplexStream, box_key: SigningKey, reply: &'static str) {
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
            _ => serde_json::json!({"version":1,"event":"link.busy","reason":"usb"}),
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
        assert_eq!(run("busy", None).await, Err(LinkError::Busy("usb".to_owned())));
    }

    #[test]
    fn the_target_needs_a_remembered_key() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-lan-target-{}-{}", std::process::id(), rand_suffix()));
        let config = LanConfig { identity: Arc::new(identity()), state_dir: dir.clone(), fixed_address: Some("127.0.0.1:7341".to_owned()) };
        assert_eq!(config.target(), None);
        remember(&box_key_file(&dir), &URL_SAFE_NO_PAD.encode([3u8; 32]));
        assert_eq!(config.target(), Some(Target { address: "127.0.0.1:7341".to_owned(), box_key: [3; 32] }));
        let config = LanConfig { fixed_address: None, ..config };
        assert_eq!(config.target(), None, "no address learned yet");
        remember(&box_address_file(&dir), "192.168.1.23");
        assert_eq!(config.target().map(|target| target.address), Some(format!("192.168.1.23:{PORT}")));
        std::fs::remove_dir_all(dir).ok();
    }
}
