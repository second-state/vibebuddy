//! Reading and writing the pack in the device's `voices` partition: a Character pack (ADR-0008),
//! or an older voice pack with five fixed lines, which still plays until the Mac replaces it. If the
//! partition is empty or fails its check, the compiled-in lines are used. Changing Character writes
//! only this partition, never the firmware (ADR-0003).
//!
//! The C firmware maps the partition into the address space and feeds I2S from it directly, so
//! the writer has to agree with the playback task on a "playing" flag first. There is no mapping
//! here: the playback task reads flash chunk by chunk, and the layer above stops playback before a
//! write session begins (see voice.begin in `Firmware`), so the two never touch this flash at once.

use crate::audio::{Chime, Codec, Line, Occasion};
use crate::character_pack::{self, CharacterPack};
use crate::lines::LinePicker;
use crate::storage::{Flash, FlashError, Region, SECTOR_BYTES};
use crate::voice_pack::{self, VoicePack};

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

/// The largest header of the two formats; the header is kept in memory and written last.
const MAX_HEADER_BYTES: usize = character_pack::HEADER_BYTES;

#[allow(clippy::large_enum_variant, reason = "there is only ever one, inside Voices; boxing it would just move it to the heap")]
enum Pack {
    Voice(VoicePack),
    Character(CharacterPack),
}

impl Pack {
    fn header_bytes(&self) -> u32 {
        match self {
            Pack::Voice(_) => voice_pack::HEADER_BYTES as u32,
            Pack::Character(_) => character_pack::HEADER_BYTES as u32,
        }
    }

    fn payload(&self) -> (u32, u32) {
        match self {
            Pack::Voice(pack) => (pack.payload_length, pack.payload_crc32),
            Pack::Character(pack) => (pack.payload_length, pack.payload_crc32),
        }
    }
}

/// How long a header is, by the magic it starts with.
fn header_bytes_for(magic: &[u8]) -> usize {
    if magic == character_pack::MAGIC { character_pack::HEADER_BYTES } else { voice_pack::HEADER_BYTES }
}

fn parse(header: &[u8], capacity: usize) -> Option<Pack> {
    if header[..4] == *character_pack::MAGIC {
        character_pack::parse(header, capacity).map(Pack::Character)
    } else {
        voice_pack::parse(header, capacity).map(Pack::Voice)
    }
}

/// The line an announcement plays, with the chime to play before it, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub line: Line,
    pub chime: Option<Chime>,
}

struct Session {
    expected_total: u32,
    received: u32,
    next_seq: u32,
    /// How much of the start is header, kept in memory; known from the first chunk's magic.
    header_bytes: usize,
    /// Leftover bytes short of 4 that cannot be written yet. Flash writes must be 4-byte aligned.
    tail: [u8; 4],
    tail_length: usize,
}

pub struct Voices {
    partition: Option<Region>,
    pack: Option<Pack>,
    session: Option<Session>,
    /// The header stays in memory and is written to flash only after the final check passes.
    header: [u8; MAX_HEADER_BYTES],
    buffer: [u8; BUFFER_BYTES + 4],
}

impl Default for Voices {
    fn default() -> Self {
        Self::new()
    }
}

impl Voices {
    pub const fn new() -> Self {
        Self { partition: None, pack: None, session: None, header: [0; MAX_HEADER_BYTES], buffer: [0; BUFFER_BYTES + 4] }
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

    /// Current Character (or voice) id; "builtin" for the built-in one.
    pub fn current_id(&self) -> &str {
        match &self.pack {
            Some(Pack::Voice(pack)) => pack.voice_id(),
            Some(Pack::Character(pack)) => pack.id(),
            None => "builtin",
        }
    }

    /// The line to play for an occasion, following the occasion's fallbacks; None means the
    /// built-in line, or silence for an occasion that has none. While a write session is running it
    /// is always None: the partition is being rewritten.
    pub fn pick(&self, occasion: Occasion, picker: &mut LinePicker) -> Option<Pick> {
        let pack = self.pack.as_ref()?;
        if self.session.is_some() {
            return None;
        }
        let partition = self.partition?;
        match pack {
            // The old voice pack: one line per ordinary occasion, chime included.
            Pack::Voice(pack) => {
                let index = occasion.builtin()? as usize;
                let line = Line {
                    offset: partition.offset + pack.clip_offset[index],
                    length: pack.clip_length[index],
                    codec: Codec::Pcm24kStereo,
                };
                Some(Pick { line, chime: None })
            }
            Pack::Character(pack) => {
                let mut wanted = Some(occasion);
                while let Some(candidate) = wanted {
                    let pool = pack.pool(candidate);
                    if !pool.is_empty() {
                        let entry = pool[picker.pick(candidate as usize, pool.len())];
                        let line = Line {
                            offset: partition.offset + entry.offset,
                            length: entry.bytes(),
                            codec: Codec::Adpcm16kMono { samples: entry.samples },
                        };
                        return Some(Pick { line, chime: candidate.chime() });
                    }
                    wanted = candidate.fallback();
                }
                None
            }
        }
    }

    /// Reads the header and verifies header and payload. A failed check treats the partition as empty.
    fn validate(&mut self, flash: &mut dyn Flash) -> bool {
        self.pack = None;
        let Some(partition) = self.partition else {
            return false;
        };
        let mut header = [0u8; MAX_HEADER_BYTES];
        if flash.read(partition.offset, &mut header).is_err() {
            return false;
        }
        let Some(parsed) = parse(&header, partition.size as usize) else {
            return false;
        };
        let (length, expected) = parsed.payload();
        match self.payload_crc(flash, partition, parsed.header_bytes(), length) {
            Ok(crc) if crc == expected => {
                self.pack = Some(parsed);
                true
            }
            _ => false,
        }
    }

    fn payload_crc(&mut self, flash: &mut dyn Flash, partition: Region, header_bytes: u32, length: u32) -> Result<u32, FlashError> {
        let mut crc = 0;
        let mut offset = 0;
        while offset < length {
            let block = (length - offset).min(BUFFER_BYTES as u32);
            // Flash reads are 4-byte aligned too; the extra tail bytes are left out of the CRC.
            let aligned = block.div_ceil(4) * 4;
            let buffer = &mut self.buffer[..aligned as usize];
            flash.read(partition.offset + header_bytes + offset, buffer)?;
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
        if total_bytes as usize <= voice_pack::HEADER_BYTES || total_bytes > partition.size {
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
        self.session = Some(Session { expected_total: total_bytes, received: 0, next_seq: 0, header_bytes: 0, tail: [0; 4], tail_length: 0 });
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

        if session.received == 0 {
            if decoded < 4 {
                return Err(VoiceError::InvalidArg);
            }
            session.header_bytes = header_bytes_for(&self.buffer[4..8]);
        }
        let mut consumed = 0;
        // The header part stays in memory for now.
        if (session.received as usize) < session.header_bytes {
            let take = (session.header_bytes - session.received as usize).min(decoded);
            let at = session.received as usize;
            self.header[at..at + take].copy_from_slice(&self.buffer[4..4 + take]);
            consumed = take;
        }
        if consumed < decoded {
            // The payload starts at 256 or 1024 and stays 4-byte aligned: the write position is the bytes
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
        let header_bytes = session.header_bytes;
        let parsed = parse(&self.header[..header_bytes], partition.size as usize).ok_or(VoiceError::InvalidResponse)?;
        let (length, expected) = parsed.payload();
        if parsed.header_bytes() + length != session.expected_total {
            return Err(VoiceError::InvalidSize);
        }
        // Read-back verification: what is in flash counts, not what is in memory.
        let crc = self.payload_crc(flash, partition, parsed.header_bytes(), length)?;
        if crc != expected {
            return Err(VoiceError::InvalidCrc);
        }
        let header = self.header;
        flash.write(partition.offset, &header[..header_bytes])?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryFlash;
    use crate::character_pack::tests::build_pack;
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
        assert_eq!(voices.pick(Occasion::Done, &mut LinePicker::new(1)), None);

        let pack = pack();
        write(&mut voices, &mut flash, &pack).unwrap();
        assert_eq!(voices.current_id(), "wanwanxiaohe");
        let start = PARTITION.offset as usize;
        assert_eq!(&flash.bytes[start..start + pack.len()], &pack[..]);
        let pick = voices.pick(Occasion::InputRequired, &mut LinePicker::new(1)).unwrap();
        assert_eq!(pick, Pick { line: Line { offset: PARTITION.offset + 256, length: 1001, codec: Codec::Pcm24kStereo }, chime: None });
        // An old voice pack speaks a special occasion with its ordinary line, and has no greeting.
        let pick = voices.pick(Occasion::FirstDone, &mut LinePicker::new(1)).unwrap();
        assert_eq!(pick.line.offset, PARTITION.offset + 256 + 1001);
        assert_eq!(voices.pick(Occasion::GreetingMorning, &mut LinePicker::new(1)), None);

        // Still recognized after a reboot.
        let mut again = Voices::new();
        again.init(&mut flash, Some(PARTITION)).unwrap();
        assert_eq!(again.current_id(), "wanwanxiaohe");
    }

    #[test]
    fn a_written_character_pack_draws_from_its_pools() {
        let mut flash = MemoryFlash::new(0x610000);
        let mut voices = Voices::new();
        voices.init(&mut flash, Some(PARTITION)).unwrap();
        // Pools: input required (2 lines), done (1), failed (none), focus done (1), break done (none),
        // first done (none), milestone (1).
        let pack = build_pack("jessica", &[&[1001, 1003], &[2001], &[], &[4001], &[], &[], &[777]]);
        write(&mut voices, &mut flash, &pack).unwrap();
        assert_eq!(voices.current_id(), "jessica");
        let mut picker = LinePicker::new(5);

        let done = voices.pick(Occasion::Done, &mut picker).unwrap();
        assert_eq!(done.line, Line { offset: PARTITION.offset + 1024 + 501 + 502, length: 1001, codec: Codec::Adpcm16kMono { samples: 2001 } });
        assert_eq!(done.chime, None);

        let milestone = voices.pick(Occasion::Milestone, &mut picker).unwrap();
        assert_eq!(milestone.line.codec, Codec::Adpcm16kMono { samples: 777 });
        assert_eq!(voices.pick(Occasion::FirstDone, &mut picker).unwrap().line, done.line, "first done falls back to done");
        assert_eq!(voices.pick(Occasion::Failed, &mut picker), None, "no failed pool: the built-in line plays");
        assert_eq!(voices.pick(Occasion::GreetingMorning, &mut picker), None);
        assert_eq!(voices.pick(Occasion::FocusDone, &mut picker).unwrap().chime, Some(Chime::Focus));

        let first = voices.pick(Occasion::InputRequired, &mut picker).unwrap().line;
        let second = voices.pick(Occasion::InputRequired, &mut picker).unwrap().line;
        assert_ne!(first, second, "a pool of two alternates");

        // Still recognized after a reboot.
        let mut again = Voices::new();
        again.init(&mut flash, Some(PARTITION)).unwrap();
        assert_eq!(again.current_id(), "jessica");
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
