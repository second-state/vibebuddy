//! Voice pack: one announcement voice's five finished lines in a single package, written
//! to the device's `voices` partition. This is the only byte layout the Mac and the
//! firmware share, and each side tests against the same sample.
//!
//! 256-byte header, little endian:
//! ```text
//!   0   magic "VBVP"
//!   4   u32 version, currently 1
//!   8   u32 payload length (bytes after the header)
//!   12  u32 payload CRC32 (same as zlib)
//!   16  char[32] voice id, NUL-terminated
//!   48  u32[5] offset of each clip from the start of the pack
//!   68  u32[5] length of each clip
//!   88  u32 CRC32 of the header's first 88 bytes
//!   92  zero padding from here on
//! ```
//! The five clips are in fixed order: input required, done, failed, focus done, break done.

pub const HEADER_BYTES: usize = 256;
pub const CLIPS: usize = 5;
pub const ID_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoicePack {
    /// NUL-terminated voice id, at most 31 bytes.
    voice_id: [u8; ID_BYTES],
    pub payload_length: u32,
    pub payload_crc32: u32,
    pub clip_offset: [u32; CLIPS],
    pub clip_length: [u32; CLIPS],
}

impl VoicePack {
    pub fn voice_id(&self) -> &str {
        let end = self.voice_id.iter().position(|&byte| byte == 0).unwrap_or(ID_BYTES);
        core::str::from_utf8(&self.voice_id[..end]).unwrap_or("?")
    }
}

fn read_u32(at: &[u8]) -> u32 {
    u32::from_le_bytes([at[0], at[1], at[2], at[3]])
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC32 as in zlib, accumulable in parts: pass 0 for the first part, then the previous result.
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = crc ^ 0xFFFF_FFFF;
    for &byte in data {
        c = CRC_TABLE[((c ^ byte as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Parses and validates the header. `capacity` is the storage area's total size; a bad
/// magic, version or header CRC, or any clip out of bounds, makes it invalid.
pub fn parse(header: &[u8], capacity: usize) -> Option<VoicePack> {
    if header.len() < HEADER_BYTES || &header[0..4] != b"VBVP" || read_u32(&header[4..]) != 1 {
        return None;
    }
    if read_u32(&header[88..]) != crc32(0, &header[..88]) {
        return None;
    }
    let mut voice_id = [0u8; ID_BYTES];
    voice_id[..ID_BYTES - 1].copy_from_slice(&header[16..16 + ID_BYTES - 1]);
    let mut pack = VoicePack {
        voice_id,
        payload_length: read_u32(&header[8..]),
        payload_crc32: read_u32(&header[12..]),
        clip_offset: [0; CLIPS],
        clip_length: [0; CLIPS],
    };
    let end_of_payload = HEADER_BYTES as u64 + pack.payload_length as u64;
    if pack.voice_id[0] == 0 || end_of_payload > capacity as u64 {
        return None;
    }
    for index in 0..CLIPS {
        let offset = read_u32(&header[48 + index * 4..]);
        let length = read_u32(&header[68 + index * 4..]);
        if length == 0 || (offset as usize) < HEADER_BYTES || offset as u64 + length as u64 > end_of_payload {
            return None;
        }
        pack.clip_offset[index] = offset;
        pack.clip_length[index] = length;
    }
    Some(pack)
}

/// Standard base64 decoding (with `=` padding) into `out`, returning the byte count. Any invalid
/// character, wrong length or output overflow returns None.
pub fn decode_base64(input: &[u8], out: &mut [u8]) -> Option<usize> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some((byte - b'A') as u32),
            b'a'..=b'z' => Some((byte - b'a' + 26) as u32),
            b'0'..=b'9' => Some((byte - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    if !input.len().is_multiple_of(4) {
        return None;
    }
    let mut written = 0;
    let quads = input.len() / 4;
    for (index, quad) in input.as_chunks::<4>().0.iter().enumerate() {
        let last = index + 1 == quads;
        let padding = quad.iter().rev().take_while(|&&byte| byte == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut bits = 0u32;
        for &byte in &quad[..4 - padding] {
            bits = (bits << 6) | value(byte)?;
        }
        bits <<= 6 * padding as u32;
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        let count = 3 - padding;
        if written + count > out.len() {
            return None;
        }
        out[written..written + count].copy_from_slice(&bytes[..count]);
        written += count;
    }
    Some(written)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put_u32(at: &mut [u8], value: u32) {
        at[..4].copy_from_slice(&value.to_le_bytes());
    }

    /// Lays out a header by hand per the spec: the five clips sit back to back after the header.
    pub(crate) fn build_header(voice_id: &str, lengths: [u32; 5], payload_crc: u32) -> [u8; HEADER_BYTES] {
        let mut header = [0u8; HEADER_BYTES];
        header[..4].copy_from_slice(b"VBVP");
        put_u32(&mut header[4..], 1);
        put_u32(&mut header[8..], lengths.iter().sum());
        put_u32(&mut header[12..], payload_crc);
        header[16..16 + voice_id.len()].copy_from_slice(voice_id.as_bytes());
        let mut offset = HEADER_BYTES as u32;
        for (index, length) in lengths.into_iter().enumerate() {
            put_u32(&mut header[48 + index * 4..], offset);
            put_u32(&mut header[68 + index * 4..], length);
            offset += length;
        }
        let crc = crc32(0, &header[..88]);
        put_u32(&mut header[88..], crc);
        header
    }

    fn reseal(header: &mut [u8; HEADER_BYTES]) {
        let crc = crc32(0, &header[..88]);
        put_u32(&mut header[88..], crc);
    }

    #[test]
    fn a_well_formed_header_parses() {
        let header = build_header("wanwanxiaohe", [1000, 2000, 3000, 4000, 5000], 0x1234_5678);
        let pack = parse(&header, 2 * 1024 * 1024).expect("should parse");
        assert_eq!(pack.voice_id(), "wanwanxiaohe");
        assert_eq!(pack.payload_length, 15000);
        assert_eq!(pack.payload_crc32, 0x1234_5678);
        assert_eq!(pack.clip_offset[0], 256);
        assert_eq!(pack.clip_length[0], 1000);
        assert_eq!(pack.clip_offset[4], 256 + 10000);
        assert_eq!(pack.clip_length[4], 5000);
    }

    #[test]
    fn crc32_matches_zlib() {
        // Standard check vector, matching Python's zlib.crc32.
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        // Accumulating in two parts gives the same value.
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn a_wrong_magic_or_version_is_rejected() {
        let mut header = build_header("x", [10; 5], 0);
        header[0] = b'X';
        assert!(parse(&header, 4096).is_none());

        let mut header = build_header("x", [10; 5], 0);
        put_u32(&mut header[4..], 2);
        reseal(&mut header);
        assert!(parse(&header, 4096).is_none());
    }

    #[test]
    fn a_corrupted_header_is_rejected() {
        let mut header = build_header("x", [10; 5], 0);
        header[20] ^= 0x01; // flip a byte in the voice id so the header CRC no longer matches
        assert!(parse(&header, 4096).is_none());
    }

    #[test]
    fn a_clip_outside_the_pack_is_rejected() {
        // The payload is larger than the storage area.
        let header = build_header("x", [10; 5], 0);
        assert!(parse(&header, 300).is_none());

        // A clip extends past the payload.
        let mut header = build_header("x", [10; 5], 0);
        put_u32(&mut header[68 + 4 * 4..], 11);
        reseal(&mut header);
        assert!(parse(&header, 4096).is_none());

        // A clip reaches into the header.
        let mut header = build_header("x", [10; 5], 0);
        put_u32(&mut header[48..], 100);
        reseal(&mut header);
        assert!(parse(&header, 4096).is_none());
    }

    #[test]
    fn base64_round_trips_standard_vectors() {
        let mut out = [0u8; 16];
        for (encoded, decoded) in [("", ""), ("Zg==", "f"), ("Zm8=", "fo"), ("Zm9v", "foo"), ("Zm9vYmFy", "foobar")] {
            let length = decode_base64(encoded.as_bytes(), &mut out).expect("valid input");
            assert_eq!(&out[..length], decoded.as_bytes());
        }
        assert!(decode_base64(b"Zm9", &mut out).is_none());
        assert!(decode_base64(b"Zg==Zm9v", &mut out).is_none());
        assert!(decode_base64(b"Zm9*", &mut out).is_none());
        assert!(decode_base64(b"Zm9vYmFy", &mut out[..5]).is_none());
    }
}
