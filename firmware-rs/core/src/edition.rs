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
//! The boot logo, if any, fills the screen. It is run-length encoded, little endian:
//! ```text
//!   0  magic "LOGO"
//!   4  u16 width (320), u16 height (240)
//!   8  runs: u16 pixel count (1 or more), u16 RGB565 color; together they cover width × height pixels exactly
//! ```
//!
//! An edition is valid only when the header and the whole pack pass their checks. An empty or
//! invalid partition, or no `edition` partition at all, counts as no edition: the box is an
//! ordinary one.

use crate::canvas;
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
    logo: Option<Region>,
}

impl Edition {
    pub fn id(&self) -> &str {
        let end = self.id.iter().position(|&byte| byte == 0).unwrap_or(ID_BYTES);
        core::str::from_utf8(&self.id[..end]).unwrap_or("?")
    }

    /// Where the boot logo is in flash; None for no logo.
    pub fn logo(&self) -> Option<Region> {
        self.logo
    }
}

const LOGO_HEADER_BYTES: u32 = 8;
const RUN_BYTES: u32 = 4;

/// Draws a boot logo into a frame, a block of runs at a time, so it never needs the whole logo in memory.
pub struct LogoDecoder {
    pixel: usize,
}

impl LogoDecoder {
    /// Checks the logo's header; None if it is not a logo for this screen.
    pub fn new(header: &[u8; LOGO_HEADER_BYTES as usize]) -> Option<Self> {
        let width = u16::from_le_bytes([header[4], header[5]]) as i32;
        let height = u16::from_le_bytes([header[6], header[7]]) as i32;
        (&header[..4] == b"LOGO" && width == canvas::WIDTH && height == canvas::HEIGHT).then_some(LogoDecoder { pixel: 0 })
    }

    /// Draws whole runs into `frame` (big-endian RGB565, as the canvas writes it). False if they go past the screen.
    pub fn feed(&mut self, runs: &[u8], frame: &mut [u8]) -> bool {
        for run in runs.as_chunks::<{ RUN_BYTES as usize }>().0 {
            let count = u16::from_le_bytes([run[0], run[1]]) as usize;
            let color = u16::from_le_bytes([run[2], run[3]]).to_be_bytes();
            if count == 0 || (self.pixel + count) * 2 > frame.len() {
                return false;
            }
            for pixel in frame[self.pixel * 2..(self.pixel + count) * 2].as_chunks_mut::<2>().0 {
                pixel.copy_from_slice(&color);
            }
            self.pixel += count;
        }
        true
    }

    /// Every pixel of the screen has been drawn.
    pub fn done(&self, frame: &[u8]) -> bool {
        self.pixel * 2 == frame.len()
    }
}

/// Where the runs of a logo stored at `logo` are, and their length; None if its length doesn't hold whole runs.
pub fn logo_runs(logo: Region) -> Option<Region> {
    let size = logo.size.checked_sub(LOGO_HEADER_BYTES)?;
    (size % RUN_BYTES == 0).then_some(Region { offset: logo.offset + LOGO_HEADER_BYTES, size })
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
    // Flash reads are 4-byte aligned, so the logo starts on a multiple of 4.
    if logo_length > 0 && (logo_offset < HEADER_BYTES as u32 || logo_offset % 4 != 0 || logo_offset as u64 + logo_length as u64 > region.size as u64) {
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

    let logo = (logo_length > 0).then_some(Region { offset: region.offset + logo_offset, size: logo_length });
    let mut edition = Edition { id: [0; ID_BYTES], logo };
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
        let edition = read(&mut flash_with(&bytes), REGION).expect("a logo inside the partition");
        assert_eq!(edition.logo(), Some(Region { offset: REGION.offset + 0x8000, size: 0x100 }));

        put_u32(&mut bytes[LOGO_AT..], 0x8002);
        reseal(&mut bytes);
        assert_eq!(read(&mut flash_with(&bytes), REGION), None, "a logo not on a multiple of 4");
        put_u32(&mut bytes[LOGO_AT..], 0x8000);

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
