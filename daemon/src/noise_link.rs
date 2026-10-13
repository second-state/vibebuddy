//! The computer's side of the Noise session on links off the cable (ADR-0015):
//! `Noise_IK_25519_ChaChaPoly_SHA256`, this computer as initiator, keyed by the pairing keys.
//!
//! The box speaks first with `link.noise`. The computer sends the first handshake message, the box the
//! second, both as lines of unpadded base64url; every line after that is one transport message, the
//! ciphertext of one protocol line. Once a box has spoken Noise the daemon remembers it ([`Memory`]) and
//! never links to it in the clear again, so whoever carries the link can't talk it back down.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, DuplexStream};

use crate::identity::Identity;

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_SHA256";
/// Bound into every handshake; the box uses the same (firmware-rs/core/src/noise.rs).
const PROLOGUE: &[u8] = b"vibebuddy-link-v2";
/// Noise's own limit on one message.
const MAX_MESSAGE: usize = 65535;
/// Longest sealed line read from the box: a screenshot line of about 2 KB is about 2.8 KB sealed.
pub const MAX_SEALED_LINE: usize = 8 * 1024;

/// The box's key from its `link.noise` line; None if the line is something else.
pub fn opener_key(line: &str) -> Option<[u8; 32]> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value["event"] != "link.noise" {
        return None;
    }
    URL_SAFE_NO_PAD.decode(value["box"].as_str()?).ok()?.try_into().ok()
}

/// A handshake started, waiting for the box's reply.
pub struct Handshake(snow::HandshakeState);

/// Starts a handshake with the box whose key is `box_key`; returns the first message as a line.
pub fn start(identity: &Identity, box_key: &[u8; 32], payload: &[u8]) -> Result<(Handshake, String), String> {
    let box_public = ed25519_dalek::VerifyingKey::from_bytes(box_key).map_err(|error| error.to_string())?.to_montgomery().to_bytes();
    let secret = identity.noise_secret();
    let params = PATTERN.parse().map_err(|error: snow::Error| error.to_string())?;
    let mut handshake = snow::Builder::new(params)
        .local_private_key(&secret)
        .and_then(|builder| builder.remote_public_key(&box_public))
        .and_then(|builder| builder.prologue(PROLOGUE))
        .and_then(snow::Builder::build_initiator)
        .map_err(|error| error.to_string())?;
    let mut message = vec![0u8; MAX_MESSAGE];
    let length = handshake.write_message(payload, &mut message).map_err(|error| error.to_string())?;
    Ok((Handshake(handshake), URL_SAFE_NO_PAD.encode(&message[..length])))
}

impl Handshake {
    /// Reads the box's reply, which proves its key, and turns to the session.
    pub fn finish(mut self, reply: &str) -> Result<Session, String> {
        let reply = URL_SAFE_NO_PAD.decode(reply.trim()).map_err(|_| "the box's reply isn't a handshake message".to_owned())?;
        let mut payload = vec![0u8; MAX_MESSAGE];
        self.0.read_message(&reply, &mut payload).map_err(|_| "the box didn't prove its key".to_owned())?;
        Ok(Session(self.0.into_transport_mode().map_err(|error| error.to_string())?))
    }
}

/// Seals lines for the box and opens the box's.
pub struct Session(snow::TransportState);

impl Session {
    pub fn seal(&mut self, line: &[u8]) -> Result<String, String> {
        let mut message = vec![0u8; line.len() + 16];
        let length = self.0.write_message(line, &mut message).map_err(|error| error.to_string())?;
        Ok(URL_SAFE_NO_PAD.encode(&message[..length]))
    }

    /// The line inside one of the box's sealed lines; an error if it was tampered with, replayed or lost.
    pub fn open(&mut self, line: &str) -> Result<Vec<u8>, String> {
        let message = URL_SAFE_NO_PAD.decode(line.trim()).map_err(|_| "not a sealed line".to_owned())?;
        let mut out = vec![0u8; message.len()];
        let length = self.0.read_message(&message, &mut out).map_err(|_| "a line that doesn't open".to_owned())?;
        out.truncate(length);
        Ok(out)
    }
}

/// Runs a session over a stream of sealed lines and returns the plain side, which carries the VibeBuddy Protocol
/// as the cable does. The plain side ends when the stream does, or on the first line that doesn't open.
pub fn bridge<S: AsyncRead + AsyncWrite + Send + 'static>(stream: S, mut session: Session) -> DuplexStream {
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let (sealed_read, mut sealed_write) = tokio::io::split(stream);
        let (plain_read, mut plain_write) = tokio::io::split(theirs);
        let mut sealed = tokio::io::BufReader::new(sealed_read);
        let mut plain = tokio::io::BufReader::new(plain_read).lines();
        let mut line = Vec::new();
        loop {
            tokio::select! {
                read = read_line(&mut sealed, &mut line) => {
                    let Ok(true) = read else { break };
                    let opened = std::str::from_utf8(&line).map_err(|_| "not a sealed line".to_owned()).and_then(|text| session.open(text));
                    let length = line.len();
                    line.clear();
                    match opened {
                        Ok(mut opened) => {
                            opened.push(b'\n');
                            if plain_write.write_all(&opened).await.is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, length, "ending the session with the box");
                            break;
                        }
                    }
                }
                outgoing = plain.next_line() => {
                    let Ok(Some(outgoing)) = outgoing else { break };
                    let Ok(sealed_line) = session.seal(outgoing.as_bytes()) else { break };
                    if sealed_write.write_all(format!("{sealed_line}\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sealed_write.shutdown().await;
        let _ = plain_write.shutdown().await;
    });
    ours
}

/// Reads the rest of one line onto `line`, without its newline; false at the end of the stream. A line longer than
/// [`MAX_SEALED_LINE`] is an error. Cancel-safe: whatever was read before a cancel stays in `line`, so the caller
/// clears it only after a whole line, never before reading.
pub async fn read_line<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R, line: &mut Vec<u8>) -> std::io::Result<bool> {
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(false);
        }
        let (taken, ended) = match buffer.iter().position(|&byte| byte == b'\n') {
            Some(end) => (end + 1, true),
            None => (buffer.len(), false),
        };
        line.extend_from_slice(&buffer[..taken]);
        reader.consume(taken);
        if line.len() > MAX_SEALED_LINE + 1 {
            return Err(std::io::Error::other("line too long"));
        }
        if ended {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(true);
        }
    }
}

/// Which box has spoken Noise to this computer, kept next to `box-key` (ADR-0015).
#[derive(Clone, Debug)]
pub struct Memory {
    path: PathBuf,
}

impl Memory {
    pub fn new(state_dir: &Path) -> Self {
        Self { path: state_dir.join("box-noise") }
    }

    /// The box with this key spoke Noise before: never link to it in the clear.
    pub fn knows(&self, box_key: &[u8; 32]) -> bool {
        std::fs::read_to_string(&self.path).is_ok_and(|text| text.trim() == URL_SAFE_NO_PAD.encode(box_key))
    }

    pub fn learned(&self, box_key: &[u8; 32]) {
        if !self.knows(box_key) {
            crate::lan::remember(&self.path, &URL_SAFE_NO_PAD.encode(box_key));
        }
    }

    /// The box on the cable no longer speaks Noise: older firmware was flashed.
    pub fn forget(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vibebuddy_firmware_core::link::Link;
    use vibebuddy_firmware_core::noise as box_noise;

    pub fn identity() -> Identity {
        let mut suffix = [0u8; 4];
        getrandom::fill(&mut suffix).unwrap();
        let dir = std::env::temp_dir().join(format!("vibebuddy-noise-test-{}-{}", std::process::id(), u32::from_le_bytes(suffix)));
        Identity::load_or_create(&dir.join("identity"), "test".to_owned()).unwrap()
    }

    fn paired_box(identity: &Identity) -> Link {
        let mut link = Link::new([1; 32]);
        let key: [u8; 32] = URL_SAFE_NO_PAD.decode(identity.public_key()).unwrap().try_into().unwrap();
        link.pair(key, "test").unwrap();
        link
    }

    #[test]
    fn the_daemon_and_the_firmware_agree() {
        let identity = identity();
        let link = paired_box(&identity);
        let (handshake, first) = start(&identity, &link.public_key(), br#"{"takeover":false}"#).unwrap();
        let accepted = box_noise::accept(&link, [4; 32], first.as_bytes()).unwrap();
        assert_eq!(accepted.payload, br#"{"takeover":false}"#);
        let mut session = handshake.finish(&accepted.reply).unwrap();
        let (mut box_sealer, mut box_opener) = (accepted.sealer, accepted.opener);

        let sealed = session.seal(br#"{"version":1,"event":"device.hello"}"#).unwrap();
        assert_eq!(box_opener.open(sealed.as_bytes()).unwrap(), br#"{"version":1,"event":"device.hello"}"#);
        let back = box_sealer.seal(b"MODE DUTY").unwrap();
        assert_eq!(session.open(&back).unwrap(), b"MODE DUTY");
        assert!(session.open(&back).is_err(), "replayed");
    }

    #[test]
    fn a_reply_from_another_box_is_refused() {
        let identity = identity();
        let link = paired_box(&identity);
        let mut impostor = Link::new([9; 32]);
        impostor.pair(link.paired[0].key, "test").unwrap();
        let (handshake, first) = start(&identity, &link.public_key(), b"{}").unwrap();
        // Only the real box can read the first message at all.
        assert!(box_noise::accept(&impostor, [4; 32], first.as_bytes()).is_err());
        assert!(handshake.finish("AAAA").is_err());
    }

    #[test]
    fn the_opener_names_the_box() {
        let link = Link::new([1; 32]);
        assert_eq!(opener_key(&box_noise::opener(&link.public_key())), Some(link.public_key()));
        assert_eq!(opener_key(r#"{"version":1,"event":"link.challenge"}"#), None);
    }

    #[tokio::test]
    async fn a_line_read_in_pieces_survives_a_cancelled_read() {
        // The bridge's select! drops the read whenever it sends; half a line must still be there next time.
        let (mut writer, reader) = tokio::io::duplex(64);
        let mut reader = tokio::io::BufReader::new(reader);
        let mut line = Vec::new();
        writer.write_all(b"half of ").await.unwrap();
        let cancelled = tokio::time::timeout(std::time::Duration::from_millis(20), read_line(&mut reader, &mut line)).await;
        assert!(cancelled.is_err());
        writer.write_all(b"a line\n").await.unwrap();
        assert!(read_line(&mut reader, &mut line).await.unwrap());
        assert_eq!(line, b"half of a line");
    }

    #[test]
    fn the_memory_is_per_box() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-noise-memory-{}", std::process::id()));
        let memory = Memory::new(&dir);
        assert!(!memory.knows(&[1; 32]));
        memory.learned(&[1; 32]);
        assert!(memory.knows(&[1; 32]));
        assert!(!memory.knows(&[2; 32]), "another box hasn't spoken Noise");
        memory.forget();
        assert!(!memory.knows(&[1; 32]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
