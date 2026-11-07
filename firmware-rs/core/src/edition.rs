//! Company edition (ADR-0025): the company Character a box of a company edition carries, in its own
//! `edition` partition that only the factory writes. Nothing in the firmware writes it, so changing
//! Character, updating the firmware or a failed write never loses it. This byte layout is shared with
//! `tools/make_edition.py`, and each side tests against the same sample.
//!
//! The partition starts with a 512-byte header, little endian:
//! ```text
//!    0  magic "VBED"
//!    4  u32 format version, 1
//!    8  char[32] edition id, NUL-terminated: the company Character's id
//!   40  char[64] the company Character's name, UTF-8, NUL-terminated
//!  104  char[256] its one-sentence summary, UTF-8, NUL-terminated
//!  360  u32 offset of the boot logo from the start of the partition, u32 its length; 0, 0 for none
//!  368  reserved, zero
//!  508  u32 CRC32 of bytes 0..508
//! ```
//! A Character pack (`VBCP`, see `character_pack.rs`) whose id is the edition id starts at 4096.
//!
//! An edition is valid only when the header and the whole pack pass their checks. An empty or
//! invalid partition, or no `edition` partition at all, counts as no edition: the box is an
//! ordinary one.

use crate::character_pack;
use crate::storage::{Flash, Region};
use crate::voice_pack::{ID_BYTES, crc32};

pub const MAGIC: &[u8; 4] = b"VBED";
pub const VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 512;
/// Where the Character pack starts, from the start of the partition.
pub const PACK_OFFSET: u32 = 4096;
const NAME_BYTES: usize = 64;
const SUMMARY_BYTES: usize = 256;
const ID_AT: usize = 8;
const NAME_AT: usize = ID_AT + ID_BYTES;
const SUMMARY_AT: usize = NAME_AT + NAME_BYTES;
const LOGO_AT: usize = SUMMARY_AT + SUMMARY_BYTES;
const CRC_AT: usize = HEADER_BYTES - 4;

/// The edition on this box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edition {
    id: [u8; ID_BYTES],
}

impl Edition {
    pub fn id(&self) -> &str {
        let end = self.id.iter().position(|&byte| byte == 0).unwrap_or(ID_BYTES);
        core::str::from_utf8(&self.id[..end]).unwrap_or("?")
    }
}

fn read_u32(at: &[u8]) -> u32 {
    u32::from_le_bytes([at[0], at[1], at[2], at[3]])
}

/// A NUL-terminated UTF-8 text field; None when it has no NUL or isn't UTF-8.
fn text(field: &[u8]) -> Option<&str> {
    let end = field.iter().position(|&byte| byte == 0)?;
    core::str::from_utf8(&field[..end]).ok()
}

/// Reads and checks the edition in `region`; None for no edition.
pub fn read(flash: &mut dyn Flash, region: Region) -> Option<Edition> {
    let mut header = [0u8; HEADER_BYTES];
    flash.read(region.offset, &mut header).ok()?;
    if &header[..4] != MAGIC || read_u32(&header[4..]) != VERSION || read_u32(&header[CRC_AT..]) != crc32(0, &header[..CRC_AT]) {
        return None;
    }
    let id = text(&header[ID_AT..NAME_AT]).filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_graphic()))?;
    text(&header[NAME_AT..SUMMARY_AT]).filter(|name| !name.is_empty())?;
    text(&header[SUMMARY_AT..LOGO_AT])?;
    let (logo_offset, logo_length) = (read_u32(&header[LOGO_AT..]), read_u32(&header[LOGO_AT + 4..]));
    if logo_length > 0 && (logo_offset < HEADER_BYTES as u32 || logo_offset as u64 + logo_length as u64 > region.size as u64) {
        return None;
    }

    if region.size <= PACK_OFFSET + character_pack::HEADER_BYTES as u32 {
        return None;
    }
    let mut pack_header = [0u8; character_pack::HEADER_BYTES];
    flash.read(region.offset + PACK_OFFSET, &mut pack_header).ok()?;
    let pack = character_pack::parse(&pack_header, (region.size - PACK_OFFSET) as usize)?;
    if pack.id() != id || payload_crc(flash, region.offset + PACK_OFFSET + character_pack::HEADER_BYTES as u32, pack.payload_length)? != pack.payload_crc32 {
        return None;
    }

    let mut edition = Edition { id: [0; ID_BYTES] };
    edition.id[..id.len()].copy_from_slice(id.as_bytes());
    Some(edition)
}

fn payload_crc(flash: &mut dyn Flash, start: u32, length: u32) -> Option<u32> {
    let mut buffer = [0u8; 1024];
    let mut crc = 0;
    let mut offset = 0;
    while offset < length {
        let block = (length - offset).min(buffer.len() as u32);
        // Flash reads are 4-byte aligned; the extra tail bytes are left out of the CRC.
        let aligned = block.div_ceil(4) * 4;
        flash.read(start + offset, &mut buffer[..aligned as usize]).ok()?;
        crc = crc32(crc, &buffer[..block as usize]);
        offset += block;
    }
    Some(crc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryFlash;
    use std::vec;
    use std::vec::Vec;

    const REGION: Region = Region { offset: 0x2000, size: 0x10000 };

    fn put_u32(at: &mut [u8], value: u32) {
        at[..4].copy_from_slice(&value.to_le_bytes());
    }

    fn reseal(header: &mut [u8]) {
        let crc = crc32(0, &header[..CRC_AT]);
        put_u32(&mut header[CRC_AT..], crc);
    }

    /// A one-line Character pack, as `tools/character_pack.py` builds it.
    fn pack(id: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; character_pack::HEADER_BYTES];
        bytes[..4].copy_from_slice(b"VBCP");
        put_u32(&mut bytes[4..], 1);
        bytes[16..16 + id.len()].copy_from_slice(id.as_bytes());
        put_u32(&mut bytes[48..], 16000);
        bytes[52] = 1;
        bytes[53] = 1;
        bytes[54] = 1;
        // input_required: line 0, one line of 10 samples (5 bytes).
        bytes[58] = 1;
        put_u32(&mut bytes[128..], character_pack::HEADER_BYTES as u32);
        put_u32(&mut bytes[132..], 10);
        let payload = [1u8, 2, 3, 4, 5];
        put_u32(&mut bytes[8..], payload.len() as u32);
        put_u32(&mut bytes[12..], crc32(0, &payload));
        let crc = crc32(0, &bytes[..1020]);
        put_u32(&mut bytes[1020..], crc);
        bytes.extend_from_slice(&payload);
        bytes
    }

    fn image(id: &str, pack_id: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; PACK_OFFSET as usize];
        bytes[..4].copy_from_slice(MAGIC);
        put_u32(&mut bytes[4..], VERSION);
        bytes[ID_AT..ID_AT + id.len()].copy_from_slice(id.as_bytes());
        bytes[NAME_AT..NAME_AT + 4].copy_from_slice(b"Acme");
        bytes[SUMMARY_AT..SUMMARY_AT + 6].copy_from_slice(b"Hello.");
        reseal(&mut bytes);
        bytes.extend(pack(pack_id));
        bytes
    }

    fn flash_with(bytes: &[u8]) -> MemoryFlash {
        let mut flash = MemoryFlash::new(0x20000);
        let start = REGION.offset as usize;
        flash.bytes[start..start + bytes.len()].copy_from_slice(bytes);
        flash
    }

    #[test]
    fn a_valid_edition_gives_its_id() {
        let edition = read(&mut flash_with(&image("acme", "acme")), REGION).expect("valid");
        assert_eq!(edition.id(), "acme");
    }

    #[test]
    fn an_empty_partition_is_no_edition() {
        assert_eq!(read(&mut MemoryFlash::new(0x20000), REGION), None);
    }

    #[test]
    fn a_damaged_header_or_pack_is_no_edition() {
        let good = image("acme", "acme");

        let mut bytes = good.clone();
        bytes[NAME_AT] ^= 1;
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "header CRC");

        let mut bytes = good.clone();
        put_u32(&mut bytes[4..], 2);
        reseal(&mut bytes);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "version");

        let mut bytes = good.clone();
        *bytes.last_mut().unwrap() ^= 1;
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "pack payload CRC");

        let mut bytes = good.clone();
        bytes.truncate(PACK_OFFSET as usize);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "no pack");

        assert_eq!(read(&mut flash_with(&image("acme", "other")), REGION), None, "the pack is another Character");
    }

    #[test]
    fn the_text_fields_must_be_terminated() {
        let mut bytes = image("acme", "acme");
        bytes[NAME_AT..SUMMARY_AT].fill(b'x');
        reseal(&mut bytes);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "name without a NUL");

        let mut bytes = image("acme", "acme");
        bytes[NAME_AT..NAME_AT + 4].fill(0);
        reseal(&mut bytes);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "empty name");
    }

    #[test]
    fn a_logo_must_fit_the_partition() {
        let mut bytes = image("acme", "acme");
        put_u32(&mut bytes[LOGO_AT..], 0x8000);
        put_u32(&mut bytes[LOGO_AT + 4..], 0x100);
        reseal(&mut bytes);
        assert!(read(&mut flash_with(&bytes), REGION).is_some(), "a logo the firmware doesn't draw yet is allowed");

        put_u32(&mut bytes[LOGO_AT + 4..], 0x10000);
        reseal(&mut bytes);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "a logo past the partition");
    }

    #[test]
    fn the_python_packer_agrees() {
        // Built by tools/make_edition.py from sample_character_pack.bin, with the name "Sample Co" and the summary "Made for tests.".
        let bytes = include_bytes!("fixtures/sample_edition.bin");
        let region = Region { offset: REGION.offset, size: 0x10000 };
        let edition = read(&mut flash_with(bytes), region).expect("valid");
        assert_eq!(edition.id(), "sample");
        assert_eq!(text(&bytes[NAME_AT..SUMMARY_AT]), Some("Sample Co"));
        assert_eq!(text(&bytes[SUMMARY_AT..LOGO_AT]), Some("Made for tests."));
    }
}
