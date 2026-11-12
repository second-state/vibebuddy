//! The two Noise handshakes of docs/vibeplay.md, VibePlay side (the responder), and the session after them.
//!
//! Pairing is `Noise_XX_25519_ChaChaPoly_SHA256` with the prologue [`PAIR_PROLOGUE`]; the link is
//! `Noise_IK_25519_ChaChaPoly_SHA256` with [`LINK_PROLOGUE`]. One handshake message per line, then one transport
//! message per line, all as unpadded base64url. The VibePlay's static key is its Ed25519 key as X25519, as the
//! daemon (`noise_link.rs`) and the box (`firmware-rs/core/src/noise.rs`) do it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clatter::bytearray::{ByteArray, SensitiveByteArray};
use clatter::cipherstate::CipherState;
use clatter::crypto::cipher::ChaChaPoly;
use clatter::crypto::dh::X25519;
use clatter::crypto::hash::Sha256;
use clatter::handshakepattern::{HandshakePattern, noise_ik, noise_xx};
use clatter::traits::{Dh, Handshaker};
use clatter::transportstate::TransportState;
use clatter::{KeyPair, NqHandshakeCore};

/// Bound into the link's handshake.
pub const LINK_PROLOGUE: &[u8] = b"vibebuddy-vibeplay-v1";
/// Bound into the pairing handshake.
pub const PAIR_PROLOGUE: &[u8] = b"vibebuddy-vibeplay-pair-v1";
/// ChaCha20-Poly1305's tag.
const TAG_BYTES: usize = 16;
/// Room for a handshake message: two keys, two tags and an empty payload, with space to spare.
const HANDSHAKE_BYTES: usize = 512;

/// The ephemeral key is always given, never drawn: QingOS gives the randomness. This only satisfies the
/// handshake's type and is never asked for anything.
#[derive(Clone, Default)]
pub struct NoRng;

impl rand_core::RngCore for NoRng {
    fn next_u32(&mut self) -> u32 {
        unreachable!("the ephemeral key is always given")
    }
    fn next_u64(&mut self) -> u64 {
        unreachable!("the ephemeral key is always given")
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        unreachable!("the ephemeral key is always given")
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        unreachable!("the ephemeral key is always given")
    }
}

impl rand_core::CryptoRng for NoRng {}

type Handshake = NqHandshakeCore<X25519, ChaChaPoly, Sha256, NoRng>;

pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim()).ok()
}

/// A 32-byte key from unpadded base64url.
pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    decode(text)?.try_into().ok()
}

fn key_pair(secret: [u8; 32]) -> KeyPair<[u8; 32], SensitiveByteArray<[u8; 32]>> {
    let secret = SensitiveByteArray::from_slice(&secret);
    KeyPair { public: X25519::pubkey(&secret), secret }
}

/// The VibePlay's side of one handshake.
pub struct Responder(Handshake);

impl Responder {
    fn new(pattern: HandshakePattern, prologue: &[u8], secret: [u8; 32], ephemeral: [u8; 32]) -> Self {
        let handshake = Handshake::new(pattern, prologue, false, Some(key_pair(secret)), Some(key_pair(ephemeral)), None, None)
            .expect("IK and XX take a static and an ephemeral key");
        Self(handshake)
    }

    /// The link: `Noise_IK`. `secret` is the VibePlay's X25519 secret; `ephemeral` must be fresh randomness.
    pub fn link(secret: [u8; 32], ephemeral: [u8; 32]) -> Self {
        Self::new(noise_ik(), LINK_PROLOGUE, secret, ephemeral)
    }

    /// Pairing: `Noise_XX`.
    pub fn pair(secret: [u8; 32], ephemeral: [u8; 32]) -> Self {
        Self::new(noise_xx(), PAIR_PROLOGUE, secret, ephemeral)
    }

    /// Reads one handshake message from a line. Its payload is empty by the protocol and is not used.
    pub fn read(&mut self, line: &str) -> Option<()> {
        let message = decode(line).filter(|message| message.len() <= HANDSHAKE_BYTES)?;
        let mut payload = vec![0u8; HANDSHAKE_BYTES];
        self.0.read_message(&message, &mut payload).ok()?;
        Some(())
    }

    /// Writes the next handshake message, with an empty payload, as a line.
    pub fn write(&mut self) -> Option<String> {
        let mut out = vec![0u8; HANDSHAKE_BYTES];
        let length = self.0.write_message(&[], &mut out).ok()?;
        Some(encode(&out[..length]))
    }

    /// The computer's X25519 static key, once a message has shown it.
    pub fn remote(&self) -> Option<[u8; 32]> {
        self.0.get_remote_static()
    }

    /// The session, once the handshake is done.
    pub fn finish(self) -> Option<Session> {
        let ciphers = TransportState::new(self.0).ok()?.take();
        Some(Session { send: ciphers.responder_to_initiator, receive: ciphers.initiator_to_responder })
    }
}

/// Both directions of a session.
pub struct Session {
    send: CipherState<ChaChaPoly>,
    receive: CipherState<ChaChaPoly>,
}

impl Session {
    /// One line, without its newline, as a transport message; None once the nonce runs out.
    pub fn seal(&mut self, line: &str) -> Option<String> {
        let mut out = vec![0u8; line.len() + TAG_BYTES];
        self.send.encrypt_with_ad(&[], line.as_bytes(), &mut out).ok()?;
        Some(encode(&out))
    }

    /// The line inside a transport message; None if it isn't one (tampered, replayed or out of order).
    pub fn open(&mut self, text: &str) -> Option<String> {
        let sealed = decode(text)?;
        let mut out = vec![0u8; sealed.len().checked_sub(TAG_BYTES)?];
        self.receive.decrypt_with_ad(&[], &sealed, &mut out).ok()?;
        String::from_utf8(out).ok()
    }
}
