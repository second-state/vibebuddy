//! Character pack: one Character's lines, a pool per occasion, in a single package written to the
//! `voices` partition (ADR-0008). Like the voice pack it replaces, this byte layout is shared with
//! the Mac (`tools/character_pack.py`), and each side tests against the same sample.
//!
//! 1024-byte header, little endian:
//! ```text
//!    0  magic "VBCP"
//!    4  u32 format version: 1, or 2 with a look
//!    8  u32 payload length (bytes after the header)
//!   12  u32 payload CRC32 (zlib)
//!   16  char[32] character id, NUL-terminated
//!   48  u32 sample rate, 16000
//!   52  u8  audio codec, 1 = IMA ADPCM, 4 bits, mono
//!   53  u8  occasion count
//!   54  u16 line count
//!   56  occasion table: per occasion, u16 first line, u16 line count (0 = no pool)
//!  128  line table: per line, u32 offset from the start of the pack, u32 sample count
//!       (at most 111 lines in version 1, 110 in version 2)
//! 1008  version 2: u32 offset of the look from the start of the pack (a multiple of 4), u32 its
//!       length; 0, 0 for none
//! 1020  u32 CRC32 of bytes 0..1020
//! ```

use crate::audio::{OCCASIONS, Occasion};
use crate::voice_pack::{ID_BYTES, crc32};

pub const MAGIC: &[u8; 4] = b"VBCP";
pub const HEADER_BYTES: usize = 1024;
pub const MAX_LINES: usize = (1020 - 128) / 8;
/// Version 2 gives up the last line slot for the look's location.
const MAX_LINES_V2: usize = (1008 - 128) / 8;
const SAMPLE_RATE: u32 = 16000;
const CODEC_IMA_ADPCM: u8 = 1;
/// Room the occasion table has before the line table starts.
const MAX_OCCASIONS: usize = (128 - 56) / 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEntry {
    /// From the start of the pack.
    pub offset: u32,
    pub samples: u32,
}

impl LineEntry {
    pub fn bytes(&self) -> u32 {
        self.samples.div_ceil(2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterPack {
    id: [u8; ID_BYTES],
    pub payload_length: u32,
    pub payload_crc32: u32,
    /// Per occasion: index of its first line and how many it has.
    pools: [(u16, u16); OCCASIONS],
    lines: [LineEntry; MAX_LINES],
    /// Where the look is, from the start of the pack, and its length.
    pub look: Option<(u32, u32)>,
}

impl CharacterPack {
    pub fn id(&self) -> &str {
        let end = self.id.iter().position(|&byte| byte == 0).unwrap_or(ID_BYTES);
        core::str::from_utf8(&self.id[..end]).unwrap_or("?")
    }

    /// The lines in this occasion's pool; empty when it has none.
    pub fn pool(&self, occasion: Occasion) -> &[LineEntry] {
        let (first, count) = self.pools[occasion as usize];
        &self.lines[first as usize..first as usize + count as usize]
    }
}

fn read_u16(at: &[u8]) -> u16 {
    u16::from_le_bytes([at[0], at[1]])
}

fn read_u32(at: &[u8]) -> u32 {
    u32::from_le_bytes([at[0], at[1], at[2], at[3]])
}

/// Parses and validates the header. `capacity` is the storage area's total size. A bad magic,
/// version, codec or header CRC, a pool past the line table, or a line outside the payload makes it
/// invalid. Occasions the pack doesn't know count as having no pool; occasions this firmware doesn't
/// know are ignored, so the table can grow.
pub fn parse(header: &[u8], capacity: usize) -> Option<CharacterPack> {
    let version = read_u32(&header[4..]);
    if header.len() < HEADER_BYTES || &header[0..4] != MAGIC || !(1..=2).contains(&version) {
        return None;
    }
    if read_u32(&header[1020..]) != crc32(0, &header[..1020]) {
        return None;
    }
    if read_u32(&header[48..]) != SAMPLE_RATE || header[52] != CODEC_IMA_ADPCM {
        return None;
    }
    let occasion_count = header[53] as usize;
    let line_count = read_u16(&header[54..]) as usize;
    if occasion_count > MAX_OCCASIONS || line_count > if version == 1 { MAX_LINES } else { MAX_LINES_V2 } {
        return None;
    }
    let mut pack = CharacterPack {
        id: [0; ID_BYTES],
        payload_length: read_u32(&header[8..]),
        payload_crc32: read_u32(&header[12..]),
        pools: [(0, 0); OCCASIONS],
        lines: [LineEntry { offset: 0, samples: 0 }; MAX_LINES],
        look: None,
    };
    pack.id[..ID_BYTES - 1].copy_from_slice(&header[16..16 + ID_BYTES - 1]);
    let end_of_payload = HEADER_BYTES as u64 + pack.payload_length as u64;
    if pack.id[0] == 0 || end_of_payload > capacity as u64 {
        return None;
    }
    for index in 0..occasion_count {
        let first = read_u16(&header[56 + index * 4..]);
        let count = read_u16(&header[58 + index * 4..]);
        if first as usize + count as usize > line_count {
            return None;
        }
        if index < OCCASIONS {
            pack.pools[index] = (first, count);
        }
    }
    for index in 0..line_count {
        let line = LineEntry { offset: read_u32(&header[128 + index * 8..]), samples: read_u32(&header[132 + index * 8..]) };
        if line.samples == 0 || (line.offset as usize) < HEADER_BYTES || line.offset as u64 + line.bytes() as u64 > end_of_payload {
            return None;
        }
        pack.lines[index] = line;
    }
    if version == 2 {
        let (offset, length) = (read_u32(&header[1008..]), read_u32(&header[1012..]));
        if length > 0 {
            // Word-aligned, so the firmware can read it from flash in one go.
            if !offset.is_multiple_of(4) || (offset as usize) < HEADER_BYTES || offset as u64 + length as u64 > end_of_payload {
                return None;
            }
            pack.look = Some((offset, length));
        }
    }
    Some(pack)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::vec::Vec;

    fn put_u16(at: &mut [u8], value: u16) {
        at[..2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(at: &mut [u8], value: u32) {
        at[..4].copy_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn reseal(header: &mut [u8]) {
        let crc = crc32(0, &header[..1020]);
        put_u32(&mut header[1020..], crc);
    }

    /// Lays out a pack by hand per the spec: `pools[occasion]` lists that pool's line sample counts,
    /// and the audio sits back to back after the header, filled with a recognizable pattern.
    pub(crate) fn build_pack(id: &str, pools: &[&[u32]]) -> Vec<u8> {
        let mut header = std::vec![0u8; HEADER_BYTES];
        header[..4].copy_from_slice(MAGIC);
        put_u32(&mut header[4..], 1);
        header[16..16 + id.len()].copy_from_slice(id.as_bytes());
        put_u32(&mut header[48..], SAMPLE_RATE);
        header[52] = CODEC_IMA_ADPCM;
        header[53] = pools.len() as u8;
        let mut line = 0usize;
        let mut offset = HEADER_BYTES as u32;
        for (occasion, pool) in pools.iter().enumerate() {
            put_u16(&mut header[56 + occasion * 4..], line as u16);
            put_u16(&mut header[58 + occasion * 4..], pool.len() as u16);
            for &samples in pool.iter() {
                put_u32(&mut header[128 + line * 8..], offset);
                put_u32(&mut header[132 + line * 8..], samples);
                offset += samples.div_ceil(2);
                line += 1;
            }
        }
        put_u16(&mut header[54..], line as u16);
        let payload: Vec<u8> = (0..offset - HEADER_BYTES as u32).map(|index| (index * 7 % 251) as u8).collect();
        put_u32(&mut header[8..], payload.len() as u32);
        put_u32(&mut header[12..], crc32(0, &payload));
        reseal(&mut header);
        header.extend_from_slice(&payload);
        header
    }

    #[test]
    fn a_well_formed_pack_parses() {
        let bytes = build_pack("jessica", &[&[100, 201], &[300], &[], &[], &[], &[51, 52, 53]]);
        let pack = parse(&bytes, 2 * 1024 * 1024).expect("should parse");
        assert_eq!(pack.id(), "jessica");
        assert_eq!(pack.pool(Occasion::InputRequired), &[
            LineEntry { offset: 1024, samples: 100 },
            LineEntry { offset: 1074, samples: 201 },
        ]);
        assert_eq!(pack.pool(Occasion::Done).len(), 1);
        assert!(pack.pool(Occasion::Failed).is_empty());
        assert_eq!(pack.pool(Occasion::FirstDone).len(), 3);
        assert!(pack.pool(Occasion::GreetingEvening).is_empty(), "occasions past the pack's table have no pool");
        assert_eq!(pack.payload_length, 50 + 101 + 150 + 26 + 26 + 27);
    }

    #[test]
    fn the_python_packer_agrees() {
        // Built by tools/character_pack.py: input required has lines of 5 and 6 samples, done one of
        // 3, the evening greeting one of 4.
        let bytes = include_bytes!("fixtures/sample_character_pack.bin");
        let pack = parse(bytes, 4096).expect("should parse");
        assert_eq!(pack.id(), "sample");
        assert_eq!(pack.pool(Occasion::InputRequired), &[LineEntry { offset: 1024, samples: 5 }, LineEntry { offset: 1027, samples: 6 }]);
        assert_eq!(pack.pool(Occasion::Done), &[LineEntry { offset: 1030, samples: 3 }]);
        assert_eq!(pack.pool(Occasion::GreetingEvening), &[LineEntry { offset: 1032, samples: 4 }]);
        assert!(pack.pool(Occasion::Failed).is_empty());
        assert_eq!(pack.payload_length as usize, bytes.len() - HEADER_BYTES);
        assert_eq!(pack.payload_crc32, crc32(0, &bytes[HEADER_BYTES..]));
    }

    #[test]
    fn a_version_2_pack_says_where_its_look_is() {
        let mut bytes = build_pack("x", &[&[10]]);
        put_u32(&mut bytes[4..], 2);
        // A look of 6 bytes after the 5-byte line, padded to a word boundary.
        bytes.extend_from_slice(b"\0\0\0LOOK!!");
        let payload_length = (bytes.len() - HEADER_BYTES) as u32;
        put_u32(&mut bytes[8..], payload_length);
        put_u32(&mut bytes[1008..], 1032);
        put_u32(&mut bytes[1012..], 6);
        reseal(&mut bytes);
        assert_eq!(parse(&bytes, 4096).expect("parses").look, Some((1032, 6)));

        put_u32(&mut bytes[1008..], 1030);
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "a look off a word boundary");
        put_u32(&mut bytes[1008..], 1032);

        put_u32(&mut bytes[1012..], 7);
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "a look past the payload");

        put_u32(&mut bytes[1012..], 0);
        reseal(&mut bytes);
        assert_eq!(parse(&bytes, 4096).expect("parses").look, None, "no look");
    }

    #[test]
    fn a_corrupted_or_foreign_header_is_rejected() {
        let good = build_pack("x", &[&[10]]);
        assert!(parse(&good, 4096).is_some());

        let mut bytes = good.clone();
        bytes[20] ^= 1;
        assert!(parse(&bytes, 4096).is_none(), "header CRC");

        let mut bytes = good.clone();
        bytes[..4].copy_from_slice(b"VBVP");
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "magic");

        let mut bytes = good.clone();
        bytes[52] = 2;
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "codec");
    }

    #[test]
    fn lines_and_pools_must_stay_in_bounds() {
        let good = build_pack("x", &[&[10, 10]]);
        assert!(parse(&good, 300).is_none(), "payload larger than the partition");

        let mut bytes = good.clone();
        put_u16(&mut bytes[58..], 3);
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "pool past the line table");

        let mut bytes = good.clone();
        put_u32(&mut bytes[136..], 1000);
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "line inside the header");

        let mut bytes = good.clone();
        put_u32(&mut bytes[140..], 13);
        reseal(&mut bytes);
        assert!(parse(&bytes, 4096).is_none(), "line past the payload");
    }
}
