//! Receiving a firmware image over any link, in chunks, resumably (ADR-0016). The computer sends
//! `xfer.begin` with the image's size and SHA-256, then `xfer.chunk`s of at most [`CHUNK_BYTES`], several
//! unacknowledged, then `xfer.end`. The box erases what the image needs in the OTA slot it isn't running from,
//! one 64 KB block per main-loop pass, before it answers `xfer.ready`: while chunks arrive it only writes, since an
//! erase stops interrupts long enough to lose bytes on the UART bridge, and block erases are far faster than
//! sector ones. It acknowledges each chunk with the next offset it expects; a chunk at any other offset is
//! acknowledged with that offset again, since every way of carrying a link delivers in order and a chunk only
//! goes missing when the link drops.
//!
//! What has been written is kept in memory, so an `xfer.begin` for the same image, over any way of carrying the
//! link, carries on from there. On `xfer.end` the box reads the slot back and checks the length and SHA-256 before
//! it touches otadata, so a transfer that fails anywhere leaves the box running what it ran.

use sha2::{Digest, Sha256};

use crate::ota;
use crate::storage::{Flash, Region, SECTOR_BYTES, find_partition};

/// What one erase step covers: a flash block, erased with one command.
const ERASE_STEP: u32 = 64 * 1024;
use crate::voice_pack::crc32;

/// Most image bytes in one chunk: as base64 inside its line, about 4.2 KB.
pub const CHUNK_BYTES: usize = 3072;

/// Why a transfer ended; the code goes to the computer as `xfer.error`'s `code`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A kind this firmware can't take, or a box without the OTA layout.
    Unsupported,
    TooBig,
    Crc,
    Sha256,
    /// Not an ESP32-S3 app image.
    BadImage,
    Flash,
    /// A chunk or an end with no transfer, or an end before the last chunk.
    Sequence,
    Cancelled,
    /// A message without the fields it needs.
    BadMessage,
}

impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::TooBig => "too_big",
            Self::Crc => "crc",
            Self::Sha256 => "sha256",
            Self::BadImage => "bad_image",
            Self::Flash => "flash",
            Self::Sequence => "sequence",
            Self::Cancelled => "cancelled",
            Self::BadMessage => "bad_message",
        }
    }
}

struct Image {
    size: u32,
    sha256: [u8; 32],
    slot: usize,
    target: Region,
    /// Bytes of the image written so far.
    next: u32,
    /// Flash erased up to here, absolute; chunks are taken once it reaches [`Image::erase_end`].
    erased_until: u32,
}

/// The transfer under way, if any.
#[derive(Default)]
pub struct Transfer {
    image: Option<Image>,
}

/// An ESP32-S3 app image starts with the magic 0xE9, and its extended header names the chip, 9 for the S3.
fn is_esp32s3_image(first: &[u8]) -> bool {
    first.len() >= 14 && first[0] == 0xE9 && first[12..14] == [9, 0]
}

impl Image {
    /// Where erasing stops: the image's end, rounded up to a whole block within the slot.
    fn erase_end(&self) -> u32 {
        (self.target.offset + self.size).next_multiple_of(ERASE_STEP).min(self.target.offset + self.target.size)
    }

    fn erased(&self) -> bool {
        self.erased_until >= self.erase_end()
    }
}

impl Transfer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> bool {
        self.image.is_some()
    }

    /// How far along, 0 to 100.
    pub fn percent(&self) -> Option<u8> {
        self.image.as_ref().map(|image| (u64::from(image.next) * 100 / u64::from(image.size.max(1))) as u8)
    }

    /// Starts a transfer, or resumes the same one. Returns the offset to send from once the slot is erased for it;
    /// None while erasing, and [`Self::erase_step`] gives the offset when done.
    pub fn begin(&mut self, flash: &mut dyn Flash, kind: &str, size: u32, sha256: [u8; 32]) -> Result<Option<u32>, Error> {
        if let Some(image) = &self.image
            && image.size == size
            && image.sha256 == sha256
            && kind == "firmware"
        {
            return Ok(image.erased().then_some(image.next));
        }
        self.image = None;
        if kind != "firmware" {
            return Err(Error::Unsupported);
        }
        let (Some(otadata), Some(ota_0), Some(ota_1)) =
            (find_partition(flash, "otadata"), find_partition(flash, "ota_0"), find_partition(flash, "ota_1"))
        else {
            return Err(Error::Unsupported);
        };
        let slot = 1 - ota::running_slot(flash, otadata).map_err(|_| Error::Flash)?;
        let target = if slot == 0 { ota_0 } else { ota_1 };
        if size == 0 || size > target.size {
            return Err(Error::TooBig);
        }
        self.image = Some(Image { size, sha256, slot, target, next: 0, erased_until: target.offset });
        Ok(None)
    }

    /// Erases the next block while a transfer is starting; the offset to send from once the last one is done.
    /// Called once per main-loop pass, so the network and the screen run between blocks.
    pub fn erase_step(&mut self, flash: &mut dyn Flash) -> Result<Option<u32>, Error> {
        let Some(image) = self.image.as_mut() else { return Ok(None) };
        if image.erased() {
            return Ok(None);
        }
        let end = (image.erased_until + ERASE_STEP).min(image.erase_end());
        if flash.erase(image.erased_until, end).is_err() {
            self.image = None;
            return Err(Error::Flash);
        }
        image.erased_until = end;
        Ok(image.erased().then_some(image.next))
    }

    /// One chunk at `offset`; returns the offset expected next. An error ends the transfer.
    pub fn chunk(&mut self, flash: &mut dyn Flash, offset: u32, data: &[u8], crc: u32) -> Result<u32, Error> {
        let result = self.write(flash, offset, data, crc);
        if result.is_err() {
            self.image = None;
        }
        result
    }

    fn write(&mut self, flash: &mut dyn Flash, offset: u32, data: &[u8], crc: u32) -> Result<u32, Error> {
        let image = self.image.as_mut().ok_or(Error::Sequence)?;
        if !image.erased() {
            return Err(Error::Sequence);
        }
        if offset != image.next {
            return Ok(image.next);
        }
        if data.is_empty() || data.len() > CHUNK_BYTES || crc32(0, data) != crc {
            return Err(if data.len() > CHUNK_BYTES { Error::TooBig } else { Error::Crc });
        }
        if offset == 0 && !is_esp32s3_image(data) {
            return Err(Error::BadImage);
        }
        let end = offset + data.len() as u32;
        if end > image.size {
            return Err(Error::TooBig);
        }
        let at = image.target.offset + offset;
        // Flash writes go in whole words; the last chunk is padded with what erased flash holds anyway.
        let mut padded = [0xFFu8; CHUNK_BYTES];
        padded[..data.len()].copy_from_slice(data);
        flash.write(at, &padded[..data.len().next_multiple_of(4)]).map_err(|_| Error::Flash)?;
        image.next = end;
        Ok(end)
    }

    /// The last chunk is in: checks the slot holds the image, then makes it boot next. The caller restarts.
    pub fn end(&mut self, flash: &mut dyn Flash) -> Result<(), Error> {
        let image = self.image.take().ok_or(Error::Sequence)?;
        if image.next != image.size {
            return Err(Error::Sequence);
        }
        let mut hasher = Sha256::new();
        let mut block = [0u8; SECTOR_BYTES as usize];
        let mut done = 0;
        while done < image.size {
            let length = (image.size - done).min(SECTOR_BYTES) as usize;
            flash.read(image.target.offset + done, &mut block[..length.next_multiple_of(4)]).map_err(|_| Error::Flash)?;
            hasher.update(&block[..length]);
            done += length as u32;
        }
        if <[u8; 32]>::from(hasher.finalize()) != image.sha256 {
            return Err(Error::Sha256);
        }
        let otadata = find_partition(flash, "otadata").ok_or(Error::Unsupported)?;
        ota::select(flash, otadata, image.slot).map_err(|_| Error::Flash)
    }

    pub fn cancel(&mut self) {
        self.image = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryFlash;
    use alloc::vec::Vec;

    /// A 16 MB flash with this firmware's partition table and blank otadata, as a USB flash leaves it.
    fn flash() -> MemoryFlash {
        let mut flash = MemoryFlash::new(0xa14000);
        let mut table = Vec::new();
        for (label, kind, subtype, offset, size) in [
            ("ota_0", 0u8, 0x10u8, 0x10000u32, 0x400000u32),
            ("ota_1", 0, 0x11, 0x610000, 0x400000),
            ("otadata", 1, 0x00, 0xa10000, 0x2000),
        ] {
            let mut entry = [0u8; 32];
            entry[0..2].copy_from_slice(&[0xAA, 0x50]);
            entry[2] = kind;
            entry[3] = subtype;
            entry[4..8].copy_from_slice(&offset.to_le_bytes());
            entry[8..12].copy_from_slice(&size.to_le_bytes());
            entry[12..12 + label.len()].copy_from_slice(label.as_bytes());
            table.extend_from_slice(&entry);
        }
        flash.bytes[0x8000..0x8000 + table.len()].copy_from_slice(&table);
        flash
    }

    fn test_image(size: usize) -> Vec<u8> {
        let mut image: Vec<u8> = (0..size).map(|index| (index * 7 + 3) as u8).collect();
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        image
    }

    fn crc(data: &[u8]) -> u32 {
        crc32(0, data)
    }

    fn sha(data: &[u8]) -> [u8; 32] {
        Sha256::digest(data).into()
    }

    /// Begins and erases, as the main loop would; returns the offset to send from.
    fn start(transfer: &mut Transfer, flash: &mut MemoryFlash, image: &[u8], sha256: [u8; 32]) -> u32 {
        if let Some(offset) = transfer.begin(flash, "firmware", image.len() as u32, sha256).unwrap() {
            return offset;
        }
        loop {
            if let Some(offset) = transfer.erase_step(flash).unwrap() {
                return offset;
            }
        }
    }

    fn send(transfer: &mut Transfer, flash: &mut MemoryFlash, image: &[u8], from: u32) -> u32 {
        let mut next = from;
        while (next as usize) < image.len() {
            let end = (next as usize + CHUNK_BYTES).min(image.len());
            let piece = &image[next as usize..end];
            next = transfer.chunk(flash, next, piece, crc(piece)).unwrap();
        }
        next
    }

    #[test]
    fn an_image_lands_in_the_other_slot_and_boots_next() {
        let mut flash = flash();
        let image = test_image(10_000);
        let mut transfer = Transfer::new();
        flash.bytes[0x610000..0x620000].fill(0);
        assert_eq!(transfer.begin(&mut flash, "firmware", image.len() as u32, sha(&image)), Ok(None), "erasing first");
        let first = &image[..CHUNK_BYTES];
        assert_eq!(transfer.chunk(&mut flash, 0, first, crc(first)), Err(Error::Sequence), "no chunks before ready");
        transfer.begin(&mut flash, "firmware", image.len() as u32, sha(&image)).unwrap();
        assert_eq!(start(&mut transfer, &mut flash, &image, sha(&image)), 0);
        assert!(flash.bytes[0x610000..0x620000].iter().all(|&byte| byte == 0xFF), "the block it needs is erased");
        assert_eq!(send(&mut transfer, &mut flash, &image, 0), 10_000);
        assert_eq!(transfer.percent(), Some(100));
        transfer.end(&mut flash).unwrap();
        assert_eq!(&flash.bytes[0x610000..0x610000 + image.len()], &image[..], "ota_1, since a USB flash runs ota_0");
        let otadata = find_partition(&mut flash, "otadata").unwrap();
        assert_eq!(ota::running_slot(&mut flash, otadata), Ok(1), "selected; the bootloader will run it");
        assert!(!transfer.active());
    }

    #[test]
    fn the_same_image_resumes_where_it_stopped() {
        let mut flash = flash();
        let image = test_image(10_000);
        let mut transfer = Transfer::new();
        start(&mut transfer, &mut flash, &image, sha(&image));
        let first = &image[..CHUNK_BYTES];
        assert_eq!(transfer.chunk(&mut flash, 0, first, crc(first)), Ok(CHUNK_BYTES as u32));
        // The link dropped and came back, perhaps another way: the same image carries on, with no erasing.
        assert_eq!(transfer.begin(&mut flash, "firmware", image.len() as u32, sha(&image)), Ok(Some(CHUNK_BYTES as u32)));
        // A chunk sent twice, or one ahead of a lost one, gets the expected offset again.
        assert_eq!(transfer.chunk(&mut flash, 0, first, crc(first)), Ok(CHUNK_BYTES as u32));
        let later = &image[2 * CHUNK_BYTES..3 * CHUNK_BYTES];
        assert_eq!(transfer.chunk(&mut flash, 2 * CHUNK_BYTES as u32, later, crc(later)), Ok(CHUNK_BYTES as u32));
        send(&mut transfer, &mut flash, &image, CHUNK_BYTES as u32);
        assert_eq!(transfer.end(&mut flash), Ok(()));

        // Another image starts over.
        let other = test_image(5_000);
        assert_eq!(start(&mut transfer, &mut flash, &other, sha(&other)), 0);
    }

    #[test]
    fn a_bad_image_a_bad_chunk_or_a_wrong_hash_never_selects_the_slot() {
        let mut flash = flash();
        let otadata = find_partition(&mut flash, "otadata").unwrap();
        let mut transfer = Transfer::new();

        let mut not_ours = test_image(4_000);
        not_ours[12] = 2;
        start(&mut transfer, &mut flash, &not_ours, sha(&not_ours));
        assert_eq!(transfer.chunk(&mut flash, 0, &not_ours[..CHUNK_BYTES], crc(&not_ours[..CHUNK_BYTES])), Err(Error::BadImage));
        assert!(!transfer.active());

        let image = test_image(4_000);
        start(&mut transfer, &mut flash, &image, sha(&image));
        assert_eq!(transfer.chunk(&mut flash, 0, &image[..CHUNK_BYTES], 1), Err(Error::Crc));

        start(&mut transfer, &mut flash, &image, [0; 32]);
        send(&mut transfer, &mut flash, &image, 0);
        assert_eq!(transfer.end(&mut flash), Err(Error::Sha256));
        assert_eq!(ota::running_slot(&mut flash, otadata), Ok(0));
        assert_eq!(flash.bytes[0xa10000..0xa10004], [0xFF; 4], "otadata untouched");
    }

    #[test]
    fn what_the_box_cannot_take_is_refused_at_begin() {
        let mut flash = flash();
        let mut transfer = Transfer::new();
        assert_eq!(transfer.begin(&mut flash, "character", 100, [0; 32]), Err(Error::Unsupported));
        assert_eq!(transfer.begin(&mut flash, "firmware", 0x400001, [0; 32]), Err(Error::TooBig));
        assert_eq!(transfer.begin(&mut MemoryFlash::new(0x10000), "firmware", 100, [0; 32]), Err(Error::Unsupported), "no OTA layout");
        assert_eq!(transfer.end(&mut flash), Err(Error::Sequence));
    }
}
