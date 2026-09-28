//! Reading and writing voice packs: the device's `voices` partition holds the current
//! announcement voice's five finished lines. If the partition is empty or fails its check, the
//! compiled-in set is used. Changing voice writes only this partition, never the firmware
//! (ADR-0003).
//!
//! The C firmware maps the partition into the address space and feeds I2S from it directly, so
//! the writer has to agree with the playback task on a "playing" flag first. There is no mapping
//! here: the playback task reads flash chunk by chunk, and the layer above stops playback before a
//! write session begins (see voice.begin in `Firmware`), so the two never touch this flash at once.

use crate::audio::Prompt;
use crate::storage::{Flash, FlashError, Region, SECTOR_BYTES};
use crate::voice_pack::{self, CLIPS, HEADER_BYTES, VoicePack};

/// Maximum raw bytes per chunk; the Mac splits by this so a base64 line stays under the protocol limit.
pub const CHUNK_BYTES: usize = 672;
/// Buffer shared by chunk decoding and read-back verification; a chunk is at most 672 bytes,
/// verification reads 1 KB at a time.
const BUFFER_BYTES: usize = 1024;

/// Why a write failed. Replies use ESP-IDF error names: the Mac and the logs have always used them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceError {
    NotFound,
    InvalidSize,
    InvalidArg,
    InvalidCrc,
    InvalidState,
    InvalidResponse,
    Fail,
    Flash,
}

impl VoiceError {
    pub fn name(self) -> &'static str {
        match self {
            VoiceError::NotFound => "ESP_ERR_NOT_FOUND",
            VoiceError::InvalidSize => "ESP_ERR_INVALID_SIZE",
            VoiceError::InvalidArg => "ESP_ERR_INVALID_ARG",
            VoiceError::InvalidCrc => "ESP_ERR_INVALID_CRC",
            VoiceError::InvalidState => "ESP_ERR_INVALID_STATE",
            VoiceError::InvalidResponse => "ESP_ERR_INVALID_RESPONSE",
            VoiceError::Fail => "ESP_FAIL",
            VoiceError::Flash => "ESP_ERR_FLASH_OP_FAIL",
        }
    }
}

impl From<FlashError> for VoiceError {
    fn from(_: FlashError) -> Self {
        VoiceError::Flash
    }
}

/// Absolute location of one line in flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipLocation {
    pub offset: u32,
    pub length: u32,
}

/// Locations of the five lines; None means the built-in voice.
pub type ClipTable = Option<[ClipLocation; CLIPS]>;

struct Session {
    expected_total: u32,
    received: u32,
    next_seq: u32,
    /// Leftover bytes short of 4 that cannot be written yet. Flash writes must be 4-byte aligned.
    tail: [u8; 4],
    tail_length: usize,
}

pub struct Voices {
    partition: Option<Region>,
    pack: Option<VoicePack>,
    session: Option<Session>,
    /// The 256 header bytes stay in memory and are written to flash only after the final check passes.
    header: [u8; HEADER_BYTES],
    buffer: [u8; BUFFER_BYTES + 4],
}

impl Default for Voices {
    fn default() -> Self {
        Self::new()
    }
}

impl Voices {
    pub const fn new() -> Self {
        Self { partition: None, pack: None, session: None, header: [0; HEADER_BYTES], buffer: [0; BUFFER_BYTES + 4] }
    }

    /// Records the partition and verifies the pack in it. Returns NotFound without one; the built-in
    /// voice still works.
    pub fn init(&mut self, flash: &mut dyn Flash, partition: Option<Region>) -> Result<(), VoiceError> {
        self.partition = partition;
        if partition.is_none() {
            return Err(VoiceError::NotFound);
        }
        self.validate(flash);
        Ok(())
    }

    /// Current voice id; "builtin" for the built-in one.
    pub fn current_id(&self) -> &str {
        match &self.pack {
            Some(pack) => pack.voice_id(),
            None => "builtin",
        }
    }

    /// Location table for playback. While a write session is running, always built-in: the partition
    /// is being rewritten.
    pub fn clips(&self) -> ClipTable {
        let pack = self.pack.as_ref()?;
        if self.session.is_some() {
            return None;
        }
        let partition = self.partition?;
        let mut table = [ClipLocation { offset: 0, length: 0 }; CLIPS];
        for (index, clip) in table.iter_mut().enumerate() {
            *clip = ClipLocation {
                offset: partition.offset + pack.clip_offset[index],
                length: pack.clip_length[index],
            };
        }
        Some(table)
    }

    /// Reads the header and verifies header and payload. A failed check treats the partition as empty.
    fn validate(&mut self, flash: &mut dyn Flash) -> bool {
        self.pack = None;
        let Some(partition) = self.partition else {
            return false;
        };
        let mut header = [0u8; HEADER_BYTES];
        if flash.read(partition.offset, &mut header).is_err() {
            return false;
        }
        let Some(parsed) = voice_pack::parse(&header, partition.size as usize) else {
            return false;
        };
        match self.payload_crc(flash, partition, parsed.payload_length) {
            Ok(crc) if crc == parsed.payload_crc32 => {
                self.pack = Some(parsed);
                true
            }
            _ => false,
        }
    }

    fn payload_crc(&mut self, flash: &mut dyn Flash, partition: Region, length: u32) -> Result<u32, FlashError> {
        let mut crc = 0;
        let mut offset = 0;
        while offset < length {
            let block = (length - offset).min(BUFFER_BYTES as u32);
            // Flash reads are 4-byte aligned too; the extra tail bytes are left out of the CRC.
            let aligned = block.div_ceil(4) * 4;
            let buffer = &mut self.buffer[..aligned as usize];
            flash.read(partition.offset + HEADER_BYTES as u32 + offset, buffer)?;
            crc = voice_pack::crc32(crc, &buffer[..block as usize]);
            offset += block;
        }
        Ok(crc)
    }

    /// Write session: begin erases the partition, chunk writes in order, end verifies before
    /// writing the header and switching. After a failure midway or an abort the partition is
    /// invalid, and announcements fall back to the built-in voice. The caller must stop playback
    /// first.
    /// Checks before begin: is there a partition, and does the total size fit.
    pub fn check_begin(&self, total_bytes: u32) -> Result<Region, VoiceError> {
        let partition = self.partition.ok_or(VoiceError::NotFound)?;
        if total_bytes as usize <= HEADER_BYTES || total_bytes > partition.size {
            return Err(VoiceError::InvalidSize);
        }
        Ok(partition)
    }

    pub fn begin(&mut self, flash: &mut dyn Flash, total_bytes: u32) -> Result<(), VoiceError> {
        let partition = self.check_begin(total_bytes)?;
        self.pack = None;
        self.session = None;
        let erase_bytes = total_bytes.div_ceil(SECTOR_BYTES) * SECTOR_BYTES;
        flash.erase(partition.offset, partition.offset + erase_bytes)?;
        self.session = Some(Session { expected_total: total_bytes, received: 0, next_seq: 0, tail: [0; 4], tail_length: 0 });
        Ok(())
    }

    /// `crc` is the CRC32 of this chunk's raw bytes: one bad byte over serial is rejected on the spot,
    /// not at the end.
    pub fn chunk(&mut self, flash: &mut dyn Flash, seq: u32, base64: &[u8], crc: u32) -> Result<(), VoiceError> {
        let partition = self.partition.ok_or(VoiceError::InvalidState)?;
        let session = self.session.as_mut().ok_or(VoiceError::InvalidState)?;
        if seq != session.next_seq {
            return Err(VoiceError::InvalidArg);
        }
        // The buffer's first 4 bytes are reserved for the previous chunk's tail; decoded bytes follow.
        let decoded = match voice_pack::decode_base64(base64, &mut self.buffer[4..4 + BUFFER_BYTES]) {
            Some(length) if length > 0 => length,
            _ => return Err(VoiceError::InvalidArg),
        };
        if session.received as usize + decoded > session.expected_total as usize {
            return Err(VoiceError::InvalidSize);
        }
        let actual = voice_pack::crc32(0, &self.buffer[4..4 + decoded]);
        if actual != crc {
            return Err(VoiceError::InvalidCrc);
        }

        let mut consumed = 0;
        // The header part stays in memory for now.
        if (session.received as usize) < HEADER_BYTES {
            let take = (HEADER_BYTES - session.received as usize).min(decoded);
            let at = session.received as usize;
            self.header[at..at + take].copy_from_slice(&self.buffer[4..4 + take]);
            consumed = take;
        }
        if consumed < decoded {
            // The payload starts at 256 and stays 4-byte aligned: the write position is the bytes
            // received so far minus the tail still being held.
            let payload_start = 4 + consumed;
            let tail_length = session.tail_length;
            let start = payload_start - tail_length;
            self.buffer[start..payload_start].copy_from_slice(&session.tail[..tail_length]);
            let pending = &self.buffer[start..4 + decoded];
            let whole = pending.len() / 4 * 4;
            let offset = session.received + consumed as u32 - tail_length as u32;
            if whole > 0 {
                flash.write(partition.offset + offset, &pending[..whole])?;
            }
            let rest = pending.len() - whole;
            session.tail[..rest].copy_from_slice(&pending[whole..]);
            session.tail_length = rest;
        }
        session.received += decoded as u32;
        session.next_seq += 1;
        Ok(())
    }

    pub fn end(&mut self, flash: &mut dyn Flash) -> Result<(), VoiceError> {
        let partition = self.partition.ok_or(VoiceError::InvalidState)?;
        let session = self.session.take().ok_or(VoiceError::InvalidState)?;
        if session.received != session.expected_total {
            return Err(VoiceError::InvalidSize);
        }
        if session.tail_length > 0 {
            let mut padded = [0xFFu8; 4];
            padded[..session.tail_length].copy_from_slice(&session.tail[..session.tail_length]);
            let offset = session.received - session.tail_length as u32;
            flash.write(partition.offset + offset, &padded)?;
        }
        let parsed = voice_pack::parse(&self.header, partition.size as usize).ok_or(VoiceError::InvalidResponse)?;
        if HEADER_BYTES as u32 + parsed.payload_length != session.expected_total {
            return Err(VoiceError::InvalidSize);
        }
        // Read-back verification: what is in flash counts, not what is in memory.
        let crc = self.payload_crc(flash, partition, parsed.payload_length)?;
        if crc != parsed.payload_crc32 {
            return Err(VoiceError::InvalidCrc);
        }
        let header = self.header;
        flash.write(partition.offset, &header)?;
        if self.validate(flash) { Ok(()) } else { Err(VoiceError::Fail) }
    }

    pub fn abort(&mut self, flash: &mut dyn Flash) {
        self.session = None;
        self.validate(flash);
    }

    pub fn writing(&self) -> bool {
        self.session.is_some()
    }
}

/// The five lines are in the same order as in the voice pack.
pub fn clip_index(prompt: Prompt) -> usize {
    prompt as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryFlash;
    use crate::voice_pack::tests::build_header;
    use std::string::String;
    use std::vec::Vec;

    const PARTITION: Region = Region { offset: 0x410000, size: 0x200000 };

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let bits = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for index in 0..4 {
                if index <= chunk.len() {
                    out.push(ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A pack whose five lines all differ in length and none is a multiple of 4, to hit the alignment
    /// edges.
    fn pack() -> Vec<u8> {
        let lengths = [1001, 2003, 3005, 4007, 5009];
        let payload: Vec<u8> = (0..lengths.iter().sum::<u32>()).map(|index| (index * 7 % 251) as u8).collect();
        let header = build_header("wanwanxiaohe", lengths, voice_pack::crc32(0, &payload));
        let mut pack = header.to_vec();
        pack.extend_from_slice(&payload);
        pack
    }

    fn write(voices: &mut Voices, flash: &mut MemoryFlash, pack: &[u8]) -> Result<(), VoiceError> {
        voices.begin(flash, pack.len() as u32)?;
        for (seq, piece) in pack.chunks(CHUNK_BYTES).enumerate() {
            let encoded = encode_base64(piece);
            voices.chunk(flash, seq as u32, encoded.as_bytes(), voice_pack::crc32(0, piece))?;
        }
        voices.end(flash)
    }

    #[test]
    fn a_written_pack_becomes_the_current_voice() {
        let mut flash = MemoryFlash::new(0x610000);
        let mut voices = Voices::new();
        voices.init(&mut flash, Some(PARTITION)).unwrap();
        assert_eq!(voices.current_id(), "builtin");
        assert_eq!(voices.clips(), None);

        let pack = pack();
        write(&mut voices, &mut flash, &pack).unwrap();
        assert_eq!(voices.current_id(), "wanwanxiaohe");
        let start = PARTITION.offset as usize;
        assert_eq!(&flash.bytes[start..start + pack.len()], &pack[..]);
        let clips = voices.clips().unwrap();
        assert_eq!(clips[0], ClipLocation { offset: PARTITION.offset + 256, length: 1001 });

        // Still recognized after a reboot.
        let mut again = Voices::new();
        again.init(&mut flash, Some(PARTITION)).unwrap();
        assert_eq!(again.current_id(), "wanwanxiaohe");
    }

    #[test]
    fn a_bad_chunk_is_rejected_and_abort_falls_back_to_builtin() {
        let mut flash = MemoryFlash::new(0x610000);
        let mut voices = Voices::new();
        voices.init(&mut flash, Some(PARTITION)).unwrap();
        let pack = pack();
        voices.begin(&mut flash, pack.len() as u32).unwrap();
        let piece = &pack[..CHUNK_BYTES];
        let encoded = encode_base64(piece);
        assert_eq!(voices.chunk(&mut flash, 1, encoded.as_bytes(), 0), Err(VoiceError::InvalidArg));
        assert_eq!(voices.chunk(&mut flash, 0, encoded.as_bytes(), 1), Err(VoiceError::InvalidCrc));
        assert_eq!(voices.chunk(&mut flash, 0, b"", 0), Err(VoiceError::InvalidArg));
        voices.abort(&mut flash);
        assert_eq!(voices.current_id(), "builtin");
        assert_eq!(voices.chunk(&mut flash, 0, encoded.as_bytes(), 0), Err(VoiceError::InvalidState));
    }

    #[test]
    fn a_short_pack_is_rejected_at_the_end() {
        let mut flash = MemoryFlash::new(0x610000);
        let mut voices = Voices::new();
        voices.init(&mut flash, Some(PARTITION)).unwrap();
        let pack = pack();
        voices.begin(&mut flash, pack.len() as u32 + 1).unwrap();
        for (seq, piece) in pack.chunks(CHUNK_BYTES).enumerate() {
            voices.chunk(&mut flash, seq as u32, encode_base64(piece).as_bytes(), voice_pack::crc32(0, piece)).unwrap();
        }
        assert_eq!(voices.end(&mut flash), Err(VoiceError::InvalidSize));
        assert_eq!(voices.current_id(), "builtin");
    }

    #[test]
    fn no_partition_means_builtin_and_not_found() {
        let mut flash = MemoryFlash::new(0x10000);
        let mut voices = Voices::new();
        assert_eq!(voices.init(&mut flash, None), Err(VoiceError::NotFound));
        assert_eq!(voices.begin(&mut flash, 1000), Err(VoiceError::NotFound));
        assert_eq!(voices.current_id(), "builtin");
    }
}
