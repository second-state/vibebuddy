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
    /// The Wi-Fi networks the box knows, most recently joined first, at most [`MAX_NETWORKS`]; set only
    /// over USB (ADR-0023).
    pub wifi: Vec<Wifi>,
    /// The Relay to use, `wss://host`, as the computer on the cable last said; None means the
    /// firmware's own default (docs/relay-protocol.md, "Which Relay").
    pub relay: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub ssid: String,
    pub password: String,
}

/// Wi-Fi's own limits: a network name is at most 32 bytes, a WPA passphrase at most 63.
pub const MAX_SSID_BYTES: usize = 32;
pub const MAX_PASSWORD_BYTES: usize = 63;
/// Most Wi-Fi networks a box remembers. Adding one more forgets the one joined longest ago.
pub const MAX_NETWORKS: usize = 5;
/// Longest Relay URL kept: its length is stored in one byte.
pub const MAX_RELAY_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairError {
    Full,
}

impl Link {
    /// A box with a new key and nobody paired.
    pub fn new(entropy: [u8; 32]) -> Self {
        Self { seed: entropy, paired: Vec::new(), wifi: Vec::new(), relay: None }
    }

    /// The box's public key, which is also its id on the Relay.
    pub fn public_key(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.seed).verifying_key().to_bytes()
    }

    /// Signs with the box's key.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(&self.seed).sign(message).to_bytes()
    }

    /// The box's key as an X25519 secret for Noise sessions (ADR-0015): the scalar behind its Ed25519 key.
    pub fn noise_secret(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.seed).to_scalar_bytes()
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

    /// Adds a network, or gives a known one its new password, and puts it first: it is the one to join now.
    /// With [`MAX_NETWORKS`] already known, the last one goes.
    pub fn remember_wifi(&mut self, ssid: &str, password: &str) {
        self.wifi.retain(|wifi| wifi.ssid != ssid);
        self.wifi.insert(0, Wifi { ssid: String::from(ssid), password: String::from(password) });
        self.wifi.truncate(MAX_NETWORKS);
    }

    /// Forgets one network. Returns whether the box knew it.
    pub fn forget_wifi(&mut self, ssid: &str) -> bool {
        let before = self.wifi.len();
        self.wifi.retain(|wifi| wifi.ssid != ssid);
        self.wifi.len() != before
    }

    /// The box joined this network: it goes first. Returns whether the order changed, so the record is saved
    /// only then.
    pub fn joined_wifi(&mut self, ssid: &str) -> bool {
        match self.wifi.iter().position(|wifi| wifi.ssid == ssid) {
            Some(index) if index > 0 => {
                let wifi = self.wifi.remove(index);
                self.wifi.insert(0, wifi);
                true
            }
            _ => false,
        }
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
        // The first network, the Relay, then the other networks are an optional tail, so a record written
        // before any of them existed still reads, and firmware from before several networks still joins the
        // first one. An empty network name means none: no network has one. An empty Relay means the default.
        let (ssid, password) = self.wifi.first().map_or(("", ""), |wifi| (wifi.ssid.as_str(), wifi.password.as_str()));
        let others = self.wifi.get(1..).unwrap_or_default();
        let mut fields = alloc::vec![ssid, password];
        if self.relay.is_some() || !others.is_empty() {
            fields.push(self.relay.as_deref().unwrap_or(""));
        } else if self.wifi.is_empty() {
            fields.clear();
        }
        for field in fields {
            payload.push(field.len() as u8);
            payload.extend_from_slice(field.as_bytes());
        }
        if !others.is_empty() {
            payload.push(others.len() as u8);
            for wifi in others {
                for field in [&wifi.ssid, &wifi.password] {
                    payload.push(field.len() as u8);
                    payload.extend_from_slice(field.as_bytes());
                }
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
        let mut wifi = Vec::new();
        if let (Some(ssid), Some(password)) = (field(), field())
            && !ssid.is_empty()
        {
            wifi.push(Wifi { ssid, password });
        }
        let relay = field().filter(|relay| !relay.is_empty());
        let others = payload.get(at).copied().unwrap_or(0) as usize;
        at += 1;
        let mut field = || -> Option<String> {
            let length = *payload.get(at)? as usize;
            let text = core::str::from_utf8(payload.get(at + 1..at + 1 + length)?).ok()?;
            at += 1 + length;
            Some(String::from(text))
        };
        for _ in 0..others {
            let (Some(ssid), Some(password)) = (field(), field()) else { break };
            if !wifi.is_empty() && !ssid.is_empty() && wifi.len() < MAX_NETWORKS && wifi.iter().all(|known| known.ssid != ssid) {
                wifi.push(Wifi { ssid, password });
            }
        }
        Some(Self { seed, paired, wifi, relay })
    }
}

/// Which known networks to try, in order, by index into `known` (most recently joined first). The ones `seen`
/// on a scan (strongest first) come first, strongest first; if none was seen, which a hidden network or a failed
/// scan also gives, every one in `known`'s order. With `first_now` the first network leads: it was just set.
pub fn join_order(known: &[&str], seen: &[&str], first_now: bool) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    if first_now && !known.is_empty() {
        order.push(0);
    }
    let visible: Vec<usize> = seen.iter().filter_map(|ssid| known.iter().position(|known| known == ssid)).collect();
    let rest = if visible.is_empty() { (0..known.len()).collect() } else { visible };
    for index in rest {
        if !order.contains(&index) {
            order.push(index);
        }
    }
    order
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

/// An Ed25519 public key as the X25519 key Noise uses (ADR-0015); None if it isn't a valid key.
pub fn noise_public(key: &[u8; 32]) -> Option<[u8; 32]> {
    VerifyingKey::from_bytes(key).ok().map(|key| key.to_montgomery().to_bytes())
}

/// Decodes canonical unpadded base64url of any length, in one pass: sealed lines are several KB.
pub fn decode_vec(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a' + 26)),
        b'0'..=b'9' => Some(u32::from(byte - b'0' + 52)),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for chunk in bytes.chunks(4) {
        let mut bits = 0u32;
        for (index, &byte) in chunk.iter().enumerate() {
            bits |= value(byte)? << (18 - 6 * index);
        }
        let produced = chunk.len() - 1;
        // The last character's spare bits must be zero for the text to be canonical.
        if produced < 3 && bits & ((1 << (8 * (3 - produced))) - 1) != 0 {
            return None;
        }
        for index in 0..produced {
            out.push((bits >> (16 - 8 * index)) as u8);
        }
    }
    Some(out)
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
    fn any_length_decodes_from_base64url() {
        for length in 0..8 {
            let bytes: Vec<u8> = (0..length).map(|index| (index * 53 + 7) as u8).collect();
            assert_eq!(decode_vec(&encode(&bytes)), Some(bytes), "{length}");
        }
        assert_eq!(decode_vec("A"), None);
        assert_eq!(decode_vec("AB"), None, "spare bits set");
        assert_eq!(decode_vec("{\"x"), None);
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
        link.remember_wifi("Home 5G", "secret pass");
        store.save(&mut flash, &link).unwrap();
        let (_, found) = LinkStore::open(&mut flash, REGION).unwrap();
        assert_eq!(found, Some(link));
    }

    #[test]
    fn the_relay_is_kept_with_or_without_a_network() {
        let mut flash = flash();
        let (mut store, _) = LinkStore::open(&mut flash, REGION).unwrap();
        let mut link = Link::new([7; 32]);
        link.relay = Some(String::from("wss://relay-staging.korekore.ai"));
        store.save(&mut flash, &link).unwrap();
        assert_eq!(LinkStore::open(&mut flash, REGION).unwrap().1, Some(link.clone()));
        link.remember_wifi("Home", "");
        store.save(&mut flash, &link).unwrap();
        assert_eq!(LinkStore::open(&mut flash, REGION).unwrap().1, Some(link));
    }

    #[test]
    fn a_record_from_before_the_relay_still_reads() {
        // seed, no computers, then the network as firmware before the Relay setting wrote it.
        let mut payload = alloc::vec![7u8; 32];
        payload.push(0);
        for field in ["Home", "pass"] {
            payload.push(field.len() as u8);
            payload.extend_from_slice(field.as_bytes());
        }
        let link = Link::decode(&payload).unwrap();
        assert_eq!(link.wifi, [Wifi { ssid: String::from("Home"), password: String::from("pass") }]);
        assert_eq!(link.relay, None);
        assert_eq!(Link::decode(&payload[..33]).unwrap().wifi, []);
    }

    fn names(link: &Link) -> Vec<&str> {
        link.wifi.iter().map(|wifi| wifi.ssid.as_str()).collect()
    }

    #[test]
    fn networks_are_remembered_newest_first_up_to_the_limit() {
        let mut link = Link::new([1; 32]);
        for name in ["A", "B", "C", "D", "E"] {
            link.remember_wifi(name, "pw");
        }
        assert_eq!(names(&link), ["E", "D", "C", "B", "A"]);
        // A known network takes its new password and goes first.
        link.remember_wifi("C", "new");
        assert_eq!(names(&link), ["C", "E", "D", "B", "A"]);
        assert_eq!(link.wifi[0].password, "new");
        // A sixth forgets the one joined longest ago.
        link.remember_wifi("F", "pw");
        assert_eq!(names(&link), ["F", "C", "E", "D", "B"]);
        assert!(link.joined_wifi("D"));
        assert!(!link.joined_wifi("D"), "already first");
        assert!(!link.joined_wifi("Z"), "not known");
        assert_eq!(names(&link), ["D", "F", "C", "E", "B"]);
        assert!(link.forget_wifi("F"));
        assert!(!link.forget_wifi("F"));
        assert_eq!(names(&link), ["D", "C", "E", "B"]);
    }

    #[test]
    fn the_box_tries_the_networks_it_hears_strongest_first() {
        let known = ["Home", "Office", "Phone"];
        assert_eq!(join_order(&known, &["Cafe", "Office", "Phone", "Office"], false), [1, 2]);
        assert_eq!(join_order(&known, &["Cafe"], false), [0, 1, 2], "none heard: newest first");
        assert_eq!(join_order(&known, &[], false), [0, 1, 2]);
        assert_eq!(join_order(&known, &["Office"], true), [0, 1], "just set: tried first");
        assert_eq!(join_order(&[], &["Office"], true), [] as [usize; 0]);
    }

    #[test]
    fn several_networks_are_kept_with_or_without_a_relay() {
        let mut flash = flash();
        let (mut store, _) = LinkStore::open(&mut flash, REGION).unwrap();
        let mut link = Link::new([7; 32]);
        link.pair([8; 32], "a").unwrap();
        for name in ["Office", "Home", "Phone"] {
            link.remember_wifi(name, &alloc::format!("{name} pass"));
        }
        store.save(&mut flash, &link).unwrap();
        assert_eq!(LinkStore::open(&mut flash, REGION).unwrap().1, Some(link.clone()));
        link.relay = Some(String::from("wss://relay-staging.korekore.ai"));
        store.save(&mut flash, &link).unwrap();
        assert_eq!(LinkStore::open(&mut flash, REGION).unwrap().1, Some(link));
    }

    #[test]
    fn firmware_from_before_several_networks_reads_the_newest_one() {
        // What firmware before several networks reads: the first network, then the Relay, and nothing after.
        let mut link = Link::new([7; 32]);
        link.remember_wifi("Office", "o");
        link.remember_wifi("Home", "h");
        let payload = link.encode();
        let mut at = 33;
        let mut field = || {
            let length = payload[at] as usize;
            at += 1 + length;
            String::from(core::str::from_utf8(&payload[at - length..at]).unwrap())
        };
        assert_eq!([field(), field(), field()], ["Home", "h", ""]);
    }

    #[test]
    fn the_largest_record_fits_a_sector() {
        let mut link = Link::new([7; 32]);
        for index in 0..MAX_PAIRED as u8 {
            link.pair([index; 32], &"n".repeat(MAX_NAME_BYTES)).unwrap();
        }
        for index in 0..MAX_NETWORKS {
            link.remember_wifi(&alloc::format!("{index}").repeat(MAX_SSID_BYTES), &"p".repeat(MAX_PASSWORD_BYTES));
        }
        link.relay = Some("r".repeat(MAX_RELAY_BYTES));
        let payload = link.encode();
        assert!(HEADER_BYTES + payload.len() + 4 <= SECTOR_BYTES as usize, "{}", payload.len());
        assert_eq!(Link::decode(&payload), Some(link));
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
