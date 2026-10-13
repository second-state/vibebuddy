//! Noise sessions on links off the cable (ADR-0015): `Noise_IK_25519_ChaChaPoly_SHA256`, the computer
//! as initiator, keyed by the Ed25519 keys exchanged at pairing.
//!
//! The box speaks first with [`opener`]. The computer answers with the first handshake message, which
//! carries its key and a payload; the box answers with the second ([`accept`]). Every line after that is
//! one transport message: the ciphertext of one protocol line, as unpadded base64url. Over the local
//! network those are lines on the TCP stream; over the Relay, the `line` of a `data` message.
//!
//! The two directions are separate [`Sealer`] and [`Opener`] halves, so the board can seal the box's
//! output while the firmware opens what comes in.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use clatter::bytearray::{ByteArray, SensitiveByteArray};
use clatter::cipherstate::CipherState;
use clatter::crypto::cipher::ChaChaPoly;
use clatter::crypto::dh::X25519;
use clatter::crypto::hash::Sha256;
use clatter::handshakepattern::noise_ik;
use clatter::traits::{Dh, Handshaker};
use clatter::transportstate::TransportState;
use clatter::{KeyPair, NqHandshakeCore};

use crate::link::{self, Link};

/// Bound into every handshake, so a session can't be mistaken for another protocol's.
pub const PROLOGUE: &[u8] = b"vibebuddy-link-v2";
/// ChaCha20-Poly1305's tag.
const TAG_BYTES: usize = 16;
/// Room for a handshake message: two keys, two tags and a small payload.
const HANDSHAKE_BYTES: usize = 512;

/// The ephemeral key is always given, never drawn: the core has no randomness of its own. This only
/// satisfies the handshake's type and is never asked for anything.
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

fn key_pair(secret: [u8; 32]) -> KeyPair<[u8; 32], SensitiveByteArray<[u8; 32]>> {
    let secret = SensitiveByteArray::from_slice(&secret);
    KeyPair { public: X25519::pubkey(&secret), secret }
}

/// The box's first line on a new link: it speaks Noise, and which box it is.
pub fn opener(box_key: &[u8; 32]) -> String {
    alloc::format!(r#"{{"version":1,"event":"link.noise","box":"{}"}}"#, link::encode(box_key))
}

/// Seals lines going one way.
pub struct Sealer(CipherState<ChaChaPoly>);

impl Sealer {
    /// One line (without its newline) as a transport message in base64url; None once the nonce runs out.
    pub fn seal(&mut self, line: &[u8]) -> Option<String> {
        let mut out = vec![0u8; line.len() + TAG_BYTES];
        self.0.encrypt_with_ad(&[], line, &mut out).ok()?;
        Some(link::encode(&out))
    }
}

/// Opens lines coming the other way.
pub struct Opener(CipherState<ChaChaPoly>);

impl Opener {
    /// The line inside a transport message; None if it isn't one (tampered, replayed, or out of order).
    pub fn open(&mut self, text: &[u8]) -> Option<Vec<u8>> {
        let sealed = link::decode_vec(core::str::from_utf8(text).ok()?)?;
        let length = sealed.len().checked_sub(TAG_BYTES)?;
        let mut out = vec![0u8; length];
        self.0.decrypt_with_ad(&[], &sealed, &mut out).ok()?;
        Some(out)
    }
}

/// The box's output on its way to a linked computer: whole lines, each sealed. The board owns it, since
/// the firmware writes through the board.
pub struct SealedLines {
    partial: Vec<u8>,
    sealer: Sealer,
}

impl SealedLines {
    pub fn new(sealer: Sealer) -> Self {
        Self { partial: Vec::new(), sealer }
    }

    /// Bytes of output: each line they complete is sealed and handed to `send`, without its newline, one at
    /// a time. A line `fits` refuses, given its sealed length, is dropped before it is sealed: a sealed line
    /// that never arrives would leave the other end unable to open any after it. `fits` is asked again for
    /// every line, after the line before it was sent.
    pub fn lines(&mut self, bytes: &[u8], mut fits: impl FnMut(usize) -> bool, mut send: impl FnMut(&str)) {
        self.partial.extend_from_slice(bytes);
        while let Some(end) = self.partial.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            if fits(sealed_len(end))
                && let Some(sealed) = self.sealer.seal(&line[..end])
            {
                send(&sealed);
            }
        }
    }

    /// One whole line, sealed, without going through the partial line.
    pub fn seal(&mut self, line: &[u8]) -> Option<String> {
        self.sealer.seal(line)
    }
}

/// How long a line of `length` bytes is once sealed: its ciphertext and tag as unpadded base64url.
pub fn sealed_len(length: usize) -> usize {
    ((length + TAG_BYTES) * 4).div_ceil(3)
}

/// Why a first handshake message was turned away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a handshake message at all.
    BadMessage,
    /// A handshake from a computer that isn't paired, or one that doesn't check out.
    Auth,
}

/// A session the box accepted.
pub struct Accepted {
    /// The paired computer at the other end, by its Ed25519 key.
    pub computer: [u8; 32],
    /// What the computer sent with its first message.
    pub payload: Vec<u8>,
    /// The second handshake message, to send as a line.
    pub reply: String,
    pub sealer: Sealer,
    pub opener: Opener,
}

/// Answers a computer's first handshake message. `ephemeral` must be fresh randomness.
pub fn accept(link: &Link, ephemeral: [u8; 32], message: &[u8]) -> Result<Accepted, Refused> {
    let message = core::str::from_utf8(message).ok().and_then(link::decode_vec).ok_or(Refused::BadMessage)?;
    if message.len() > HANDSHAKE_BYTES {
        return Err(Refused::BadMessage);
    }
    let mut handshake =
        Handshake::new(noise_ik(), PROLOGUE, false, Some(key_pair(link.noise_secret())), Some(key_pair(ephemeral)), None, None)
            .map_err(|_| Refused::Auth)?;
    let mut payload = vec![0u8; HANDSHAKE_BYTES];
    let length = handshake.read_message(&message, &mut payload).map_err(|_| Refused::Auth)?;
    payload.truncate(length);
    let remote = handshake.get_remote_static().ok_or(Refused::Auth)?;
    let computer = link
        .paired
        .iter()
        .find(|paired| link::noise_public(&paired.key) == Some(remote))
        .map(|paired| paired.key)
        .ok_or(Refused::Auth)?;
    let mut reply = vec![0u8; HANDSHAKE_BYTES];
    let length = handshake.write_message(&[], &mut reply).map_err(|_| Refused::Auth)?;
    reply.truncate(length);
    let ciphers = TransportState::new(handshake).map_err(|_| Refused::Auth)?.take();
    Ok(Accepted {
        computer,
        payload,
        reply: link::encode(&reply),
        sealer: Sealer(ciphers.responder_to_initiator),
        opener: Opener(ciphers.initiator_to_responder),
    })
}

/// The computer's side, for the simulator's tests and the firmware's: the daemon has its own.
pub struct Initiator(Handshake);

impl Initiator {
    /// Starts a handshake with the box whose key is `box_key`; returns the first message as a line.
    pub fn start(computer_seed: [u8; 32], box_key: &[u8; 32], ephemeral: [u8; 32], payload: &[u8]) -> Option<(Self, String)> {
        let secret = ed25519_dalek::SigningKey::from_bytes(&computer_seed).to_scalar_bytes();
        let box_public = link::noise_public(box_key)?;
        let mut handshake =
            Handshake::new(noise_ik(), PROLOGUE, true, Some(key_pair(secret)), Some(key_pair(ephemeral)), Some(box_public), None).ok()?;
        let mut out = vec![0u8; HANDSHAKE_BYTES];
        let length = handshake.write_message(payload, &mut out).ok()?;
        out.truncate(length);
        Some((Self(handshake), link::encode(&out)))
    }

    /// Reads the box's reply: this computer's sealer (to the box) and opener (from it).
    pub fn finish(mut self, reply: &[u8]) -> Option<(Sealer, Opener)> {
        let reply = link::decode_vec(core::str::from_utf8(reply).ok()?)?;
        let mut payload = vec![0u8; HANDSHAKE_BYTES];
        self.0.read_message(&reply, &mut payload).ok()?;
        let ciphers = TransportState::new(self.0).ok()?.take();
        Some((Sealer(ciphers.initiator_to_responder), Opener(ciphers.responder_to_initiator)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPUTER: [u8; 32] = [2; 32];

    fn paired_box() -> Link {
        let mut link = Link::new([1; 32]);
        let computer = ed25519_dalek::SigningKey::from_bytes(&COMPUTER).verifying_key().to_bytes();
        link.pair(computer, "a").unwrap();
        link
    }

    #[test]
    fn the_converted_keys_agree() {
        // X25519 on the Ed25519 scalar gives the Montgomery form of the Ed25519 public key.
        let link = Link::new([7; 32]);
        assert_eq!(Some(X25519::pubkey(&SensitiveByteArray::from_slice(&link.noise_secret()))), link::noise_public(&link.public_key()));
    }

    #[test]
    fn a_paired_computer_gets_a_session_both_ways() {
        let link = paired_box();
        let (initiator, first) = Initiator::start(COMPUTER, &link.public_key(), [3; 32], br#"{"takeover":true}"#).unwrap();
        let accepted = accept(&link, [4; 32], first.as_bytes()).unwrap();
        assert_eq!(accepted.computer, link.paired[0].key);
        assert_eq!(accepted.payload, br#"{"takeover":true}"#);
        let (mut to_box, mut from_box) = initiator.finish(accepted.reply.as_bytes()).unwrap();
        let (mut box_sealer, mut box_opener) = (accepted.sealer, accepted.opener);

        let line = to_box.seal(br#"{"version":1,"event":"device.hello"}"#).unwrap();
        assert!(!line.contains("device.hello"), "the Relay sees ciphertext");
        assert_eq!(box_opener.open(line.as_bytes()).unwrap(), br#"{"version":1,"event":"device.hello"}"#);
        let back = box_sealer.seal(b"MODE DUTY").unwrap();
        assert_eq!(from_box.open(back.as_bytes()).unwrap(), b"MODE DUTY");
    }

    #[test]
    fn output_is_sealed_line_by_line_and_a_line_that_does_not_fit_is_never_sealed() {
        let link = paired_box();
        let (initiator, first) = Initiator::start(COMPUTER, &link.public_key(), [3; 32], b"{}").unwrap();
        let accepted = accept(&link, [4; 32], first.as_bytes()).unwrap();
        let (_, mut from_box) = initiator.finish(accepted.reply.as_bytes()).unwrap();
        let mut lines = SealedLines::new(accepted.sealer);
        let mut sealed = Vec::new();
        lines.lines(b"MODE ", |_| true, |line| sealed.push(String::from(line)));
        assert!(sealed.is_empty());
        lines.lines(b"DUTY\nDROPPED\nVOLUME 65\n", |length| length != sealed_len(7), |line| sealed.push(String::from(line)));
        assert_eq!(sealed.len(), 2);
        assert_eq!(sealed[0].len(), sealed_len(9));
        assert_eq!(from_box.open(sealed[0].as_bytes()).unwrap(), b"MODE DUTY");
        assert_eq!(from_box.open(sealed[1].as_bytes()).unwrap(), b"VOLUME 65", "the dropped line cost no nonce");
    }

    #[test]
    fn replayed_tampered_or_reordered_lines_are_refused() {
        let link = paired_box();
        let (initiator, first) = Initiator::start(COMPUTER, &link.public_key(), [3; 32], b"{}").unwrap();
        let accepted = accept(&link, [4; 32], first.as_bytes()).unwrap();
        let (mut to_box, _) = initiator.finish(accepted.reply.as_bytes()).unwrap();
        let mut opener = accepted.opener;
        let one = to_box.seal(b"one").unwrap();
        let two = to_box.seal(b"two").unwrap();
        assert_eq!(opener.open(two.as_bytes()), None, "out of order");

        let mut opener = accept(&link, [4; 32], first.as_bytes()).unwrap().opener;
        assert_eq!(opener.open(one.as_bytes()).unwrap(), b"one");
        assert_eq!(opener.open(one.as_bytes()), None, "replayed");
        let mut tampered = two.into_bytes();
        tampered[3] = if tampered[3] == b'A' { b'B' } else { b'A' };
        assert_eq!(opener.open(&tampered), None);
    }

    #[test]
    fn an_unpaired_computer_or_garbage_is_turned_away() {
        let link = paired_box();
        let (_, stranger) = Initiator::start([9; 32], &link.public_key(), [3; 32], b"{}").unwrap();
        assert!(matches!(accept(&link, [4; 32], stranger.as_bytes()), Err(Refused::Auth)));
        let (_, for_another_box) = Initiator::start(COMPUTER, &Link::new([8; 32]).public_key(), [3; 32], b"{}").unwrap();
        assert!(matches!(accept(&link, [4; 32], for_another_box.as_bytes()), Err(Refused::Auth)));
        assert!(matches!(accept(&link, [4; 32], br#"{"version":1,"event":"link.hello"}"#), Err(Refused::BadMessage)));
    }
}
