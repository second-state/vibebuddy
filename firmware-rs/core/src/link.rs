//! The box's side of pairing (ADR-0012): its own Ed25519 key, made from hardware entropy the first
//! time it boots this firmware, and the computers paired with it. Both live in the `link` partition,
//! which no flash of firmware touches, so a box keeps its identity and its pairings across updates.
//!
//! The partition is two sectors, each able to hold one whole record; a save writes the sector the
//! latest record isn't in, so losing power halfway leaves the previous record intact. Saves happen
//! only when pairing changes.

use alloc::string::String;
use alloc::vec::Vec;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::storage::{Flash, FlashError, Region, SECTOR_BYTES};
use crate::voice_pack::crc32;

/// Most computers a box remembers, the same limit the Relay enforces.
pub const MAX_PAIRED: usize = 16;
/// Longest computer name kept, in bytes; longer names are cut at a character boundary.
pub const MAX_NAME_BYTES: usize = 32;

const MAGIC: u32 = u32::from_le_bytes(*b"VBL1");
/// magic, sequence, payload length, then the payload, then a CRC over all of it.
const HEADER_BYTES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    pub key: [u8; 32],
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The secret seed of the box's key.
    seed: [u8; 32],
    pub paired: Vec<Paired>,
    /// The Wi-Fi network to join, name and password; set only over USB.
    pub wifi: Option<Wifi>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    pub password: String,
}

/// Wi-Fi's own limits: a network name is at most 32 bytes, a WPA passphrase at most 63.
pub const MAX_SSID_BYTES: usize = 32;
pub const MAX_PASSWORD_BYTES: usize = 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairError {
    Full,
}

impl Link {
    /// A box with a new key and nobody paired.
    pub fn new(entropy: [u8; 32]) -> Self {
        Self { seed: entropy, paired: Vec::new(), wifi: None }
    }

    /// The box's public key, which is also its id on the Relay.
    pub fn public_key(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.seed).verifying_key().to_bytes()
    }

    /// Signs with the box's key.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(&self.seed).sign(message).to_bytes()
    }

    pub fn is_paired(&self, key: &[u8; 32]) -> bool {
        self.paired.iter().any(|paired| &paired.key == key)
    }

    /// Adds a computer, or renames one already paired. Returns whether anything changed.
    pub fn pair(&mut self, key: [u8; 32], name: &str) -> Result<bool, PairError> {
        let name = String::from(truncate(name, MAX_NAME_BYTES));
        if let Some(existing) = self.paired.iter_mut().find(|paired| paired.key == key) {
            if existing.name == name {
                return Ok(false);
            }
            existing.name = name;
            return Ok(true);
        }
        if self.paired.len() >= MAX_PAIRED {
            return Err(PairError::Full);
        }
        self.paired.push(Paired { key, name });
        Ok(true)
    }

    /// Forgets a computer. Returns whether it was paired.
    pub fn unpair(&mut self, key: &[u8; 32]) -> bool {
        let before = self.paired.len();
        self.paired.retain(|paired| &paired.key != key);
        self.paired.len() != before
    }

    fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(33 + self.paired.len() * (33 + MAX_NAME_BYTES));
        payload.extend_from_slice(&self.seed);
        payload.push(self.paired.len() as u8);
        for paired in &self.paired {
            payload.extend_from_slice(&paired.key);
            payload.push(paired.name.len() as u8);
            payload.extend_from_slice(paired.name.as_bytes());
        }
        // The network is an optional tail, so a record written before it existed still reads.
        if let Some(wifi) = &self.wifi {
            for field in [&wifi.ssid, &wifi.password] {
                payload.push(field.len() as u8);
                payload.extend_from_slice(field.as_bytes());
            }
        }
        payload
    }

    fn decode(payload: &[u8]) -> Option<Self> {
        let seed: [u8; 32] = payload.get(..32)?.try_into().ok()?;
        let count = *payload.get(32)? as usize;
        let mut at = 33;
        let mut paired = Vec::with_capacity(count);
        for _ in 0..count.min(MAX_PAIRED) {
            let key: [u8; 32] = payload.get(at..at + 32)?.try_into().ok()?;
            let length = *payload.get(at + 32)? as usize;
            let name = core::str::from_utf8(payload.get(at + 33..at + 33 + length)?).ok()?;
            paired.push(Paired { key, name: String::from(name) });
            at += 33 + length;
        }
        let mut field = || -> Option<String> {
            let length = *payload.get(at)? as usize;
            let text = core::str::from_utf8(payload.get(at + 1..at + 1 + length)?).ok()?;
            at += 1 + length;
            Some(String::from(text))
        };
        let wifi = match (field(), field()) {
            (Some(ssid), Some(password)) => Some(Wifi { ssid, password }),
            _ => None,
        };
        Some(Self { seed, paired, wifi })
    }
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub struct LinkStore {
    region: Region,
    /// Sector and sequence number of the latest valid record.
    latest: Option<(u32, u32)>,
}

impl LinkStore {
    /// Reads both sectors, returning the store and the latest link found (None on a box that has never had one).
    pub fn open(flash: &mut dyn Flash, region: Region) -> Result<(Self, Option<Link>), FlashError> {
        let mut store = Self { region, latest: None };
        let mut found = None;
        if region.size < 2 * SECTOR_BYTES {
            return Err(FlashError);
        }
        for sector in 0..2 {
            let offset = region.offset + sector * SECTOR_BYTES;
            let mut header = [0u8; HEADER_BYTES];
            flash.read(offset, &mut header)?;
            let word = |index: usize| u32::from_le_bytes(header[index * 4..index * 4 + 4].try_into().unwrap());
            let length = word(2) as usize;
            if word(0) != MAGIC || HEADER_BYTES + length + 4 > SECTOR_BYTES as usize {
                continue;
            }
            let mut record = alloc::vec![0u8; (HEADER_BYTES + length + 4).div_ceil(4) * 4];
            flash.read(offset, &mut record)?;
            let crc = u32::from_le_bytes(record[HEADER_BYTES + length..HEADER_BYTES + length + 4].try_into().unwrap());
            if crc != crc32(0, &record[..HEADER_BYTES + length]) {
                continue;
            }
            let Some(link) = Link::decode(&record[HEADER_BYTES..HEADER_BYTES + length]) else { continue };
            let sequence = word(1);
            let newer = match store.latest {
                None => true,
                Some((_, latest)) => sequence.wrapping_sub(latest) as i32 > 0,
            };
            if newer {
                store.latest = Some((sector, sequence));
                found = Some(link);
            }
        }
        Ok((store, found))
    }

    pub fn save(&mut self, flash: &mut dyn Flash, link: &Link) -> Result<(), FlashError> {
        let (sector, sequence) = match self.latest {
            Some((sector, sequence)) => (1 - sector, sequence.wrapping_add(1)),
            None => (0, 1),
        };
        let payload = link.encode();
        let mut record = Vec::with_capacity(HEADER_BYTES + payload.len() + 8);
        record.extend_from_slice(&MAGIC.to_le_bytes());
        record.extend_from_slice(&sequence.to_le_bytes());
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(&payload);
        let crc = crc32(0, &record);
        record.extend_from_slice(&crc.to_le_bytes());
        record.resize(record.len().div_ceil(4) * 4, 0xFF);
        let offset = self.region.offset + sector * SECTOR_BYTES;
        flash.erase(offset, offset + SECTOR_BYTES)?;
        flash.write(offset, &record)?;
        self.latest = Some((sector, sequence));
        Ok(())
    }
}

/// Unpadded base64url, the way keys, nonces and signatures travel in messages and lines.
pub fn encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |acc, (index, &byte)| acc | u32::from(byte) << (16 - 8 * index));
        for index in 0..=chunk.len() {
            out.push(ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    out
}

/// Decodes exactly `N` bytes of canonical unpadded base64url.
pub fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N.div_ceil(3) * 4 - (3 - N % 3) % 3 {
        return None;
    }
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    };
    let mut out = [0u8; N];
    let mut at = 0;
    for chunk in text.as_bytes().chunks(4) {
        let mut bits = 0u32;
        for (index, &byte) in chunk.iter().enumerate() {
            bits |= u32::from(value(byte)?) << (18 - 6 * index);
        }
        for index in 0..chunk.len() - 1 {
            out[at] = (bits >> (16 - 8 * index)) as u8;
            at += 1;
        }
    }
    // The last character's spare bits must be zero for the text to be canonical.
    (encode(&out) == text).then_some(out)
}

pub fn encode_key(key: &[u8; 32]) -> String {
    encode(key)
}

pub fn decode_key(text: &str) -> Option<[u8; 32]> {
    decode(text)
}

/// Checks `signature` by `key` over `message`.
pub fn verify(key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(key).is_ok_and(|key| key.verify_strict(message, &Signature::from_bytes(signature)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryFlash;

    const REGION: Region = Region { offset: 0x2000, size: 0x2000 };

    fn flash() -> MemoryFlash {
        MemoryFlash::new(0x4000)
    }

    #[test]
    fn keys_round_trip_through_base64url() {
        let key: [u8; 32] = core::array::from_fn(|index| (index * 37 + 5) as u8);
        let text = encode_key(&key);
        assert_eq!(text.len(), 43);
        assert!(text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
        assert_eq!(decode_key(&text), Some(key));
        assert_eq!(encode_key(&[0; 32]), "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    }

    #[test]
    fn signatures_round_trip_through_base64url_and_verify() {
        let link = Link::new([3; 32]);
        let signature = link.sign(b"hello");
        let text = encode(&signature);
        assert_eq!(text.len(), 86);
        assert_eq!(decode::<64>(&text), Some(signature));
        assert!(verify(&link.public_key(), b"hello", &signature));
        assert!(!verify(&link.public_key(), b"hellO", &signature));
    }

    #[test]
    fn decoding_rejects_what_isnt_a_key() {
        assert_eq!(decode_key("short"), None);
        assert_eq!(decode_key(&"+".repeat(43)), None);
        // Nonzero spare bits in the last character.
        assert_eq!(decode_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB"), None);
    }

    #[test]
    fn the_public_key_follows_from_the_seed() {
        // RFC 8032 test 1.
        let mut seed = [0u8; 32];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&"9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"[index * 2..index * 2 + 2], 16).unwrap();
        }
        let public = Link::new(seed).public_key();
        assert_eq!(public[..4], [0xd7, 0x5a, 0x98, 0x01]);
    }

    #[test]
    fn pairing_adds_renames_and_fills_up() {
        let mut link = Link::new([1; 32]);
        assert_eq!(link.pair([2; 32], "MacBook"), Ok(true));
        assert_eq!(link.pair([2; 32], "MacBook"), Ok(false));
        assert_eq!(link.pair([2; 32], "dragon's MacBook"), Ok(true));
        assert_eq!(link.paired.len(), 1);
        for index in 1..MAX_PAIRED as u8 {
            link.pair([index + 2; 32], "x").unwrap();
        }
        assert_eq!(link.pair([99; 32], "one too many"), Err(PairError::Full));
        assert!(link.unpair(&[2; 32]));
        assert!(!link.unpair(&[2; 32]));
    }

    #[test]
    fn long_names_are_cut_at_a_character_boundary() {
        let mut link = Link::new([1; 32]);
        link.pair([2; 32], &"盒".repeat(20)).unwrap();
        assert_eq!(link.paired[0].name, "盒".repeat(10));
    }

    #[test]
    fn an_empty_partition_has_no_link() {
        let mut flash = flash();
        let (_, found) = LinkStore::open(&mut flash, REGION).unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn saves_alternate_sectors_and_reopen_to_the_latest() {
        let mut flash = flash();
        let (mut store, _) = LinkStore::open(&mut flash, REGION).unwrap();
        let mut link = Link::new([7; 32]);
        store.save(&mut flash, &link).unwrap();
        link.pair([8; 32], "first").unwrap();
        store.save(&mut flash, &link).unwrap();
        link.pair([9; 32], "second").unwrap();
        store.save(&mut flash, &link).unwrap();
        assert_eq!(flash.erases, [(0x2000, 0x3000), (0x3000, 0x4000), (0x2000, 0x3000)]);

        let (_, found) = LinkStore::open(&mut flash, REGION).unwrap();
        assert_eq!(found, Some(link));
    }

    #[test]
    fn the_wifi_network_is_kept_with_the_rest() {
        let mut flash = flash();
        let (mut store, _) = LinkStore::open(&mut flash, REGION).unwrap();
        let mut link = Link::new([7; 32]);
        link.pair([8; 32], "a").unwrap();
        link.wifi = Some(Wifi { ssid: String::from("Home 5G"), password: String::from("secret pass") });
        store.save(&mut flash, &link).unwrap();
        let (_, found) = LinkStore::open(&mut flash, REGION).unwrap();
        assert_eq!(found, Some(link));
    }

    #[test]
    fn a_torn_save_leaves_the_previous_record() {
        let mut flash = flash();
        let (mut store, _) = LinkStore::open(&mut flash, REGION).unwrap();
        let mut link = Link::new([7; 32]);
        link.pair([8; 32], "kept").unwrap();
        store.save(&mut flash, &link).unwrap();
        let previous = link.clone();
        link.pair([9; 32], "lost").unwrap();
        store.save(&mut flash, &link).unwrap();
        // Power lost after the erase and part of the write of the second sector.
        flash.bytes[0x3000 + 20..0x4000].fill(0xFF);

        let (_, found) = LinkStore::open(&mut flash, REGION).unwrap();
        assert_eq!(found, Some(previous));
    }
}
