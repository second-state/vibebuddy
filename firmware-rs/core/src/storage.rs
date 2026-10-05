//! On-chip flash: partition table and settings records.
//!
//! The C firmware keeps today's pomodoro tally and the volume in ESP-IDF's NVS. esp-hal has no NVS,
//! so this writes a small append-only log over the old `nvs` partition: each record is 32 bytes
//! with its own sequence number and CRC, and reading takes the valid record with the highest
//! sequence. There are only a few writes a day, and appending spreads erases out to one per 128
//! records, so flash does not wear. When the first boot finds no valid record (the old NVS pages
//! are still there), it is treated as a brand-new device and erasing starts from the first sector.

use crate::pomodoro::Tally;
use crate::voice_pack::crc32;

pub const SECTOR_BYTES: u32 = 4096;

/// A flash operation failed. ESP-IDF calls it ESP_ERR_FLASH_OP_FAIL, and replies keep that name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashError;

/// The device's flash. Offsets are absolute addresses in the whole flash; write offsets and lengths
/// must be multiples of 4, and erases are sector-aligned.
pub trait Flash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError>;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError>;
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError>;
}

/// A region of flash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub offset: u32,
    pub size: u32,
}

pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;
const PARTITION_ENTRY_BYTES: usize = 32;
const PARTITION_MAX_ENTRIES: usize = 95;

/// Finds a partition by label. ESP-IDF partition table: 32 bytes per entry, magic 0xAA 0x50, then
/// type, subtype, u32 offset, u32 size, 16-byte label, u32 flags; ends at an all-0xFF or MD5 entry.
pub fn find_partition(flash: &mut dyn Flash, label: &str) -> Option<Region> {
    let mut entry = [0u8; PARTITION_ENTRY_BYTES];
    for index in 0..PARTITION_MAX_ENTRIES {
        let offset = PARTITION_TABLE_OFFSET + (index * PARTITION_ENTRY_BYTES) as u32;
        flash.read(offset, &mut entry).ok()?;
        if entry[0] != 0xAA || entry[1] != 0x50 {
            return None;
        }
        let name = &entry[12..28];
        let name_end = name.iter().position(|&byte| byte == 0).unwrap_or(name.len());
        if &name[..name_end] == label.as_bytes() {
            return Some(Region {
                offset: u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]),
                size: u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]),
            });
        }
    }
    None
}

/// What the device remembers itself: today's pomodoro tally and the volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub tally: Tally,
    pub volume: u32,
}

const RECORD_BYTES: u32 = 32;
const RECORD_MAGIC: u32 = u32::from_le_bytes(*b"VBS1");
const SLOTS_PER_SECTOR: u32 = SECTOR_BYTES / RECORD_BYTES;

pub struct SettingsStore {
    region: Region,
    /// Slot and sequence number of the latest valid record.
    latest: Option<(u32, u32)>,
}

fn encode(settings: &Settings, sequence: u32) -> [u8; RECORD_BYTES as usize] {
    let mut record = [0xFFu8; RECORD_BYTES as usize];
    let words = [
        RECORD_MAGIC,
        sequence,
        settings.tally.day,
        settings.tally.completed,
        settings.tally.focus_s,
        settings.volume,
    ];
    for (index, word) in words.iter().enumerate() {
        record[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    let crc = crc32(0, &record[..28]);
    record[28..].copy_from_slice(&crc.to_le_bytes());
    record
}

fn decode(record: &[u8; RECORD_BYTES as usize]) -> Option<(u32, Settings)> {
    let word = |index: usize| u32::from_le_bytes([record[index * 4], record[index * 4 + 1], record[index * 4 + 2], record[index * 4 + 3]]);
    if word(0) != RECORD_MAGIC || word(7) != crc32(0, &record[..28]) {
        return None;
    }
    let tally = Tally { day: word(2), completed: word(3), focus_s: word(4) };
    Some((word(1), Settings { tally, volume: word(5) }))
}

impl SettingsStore {
    /// Scans the region, returning the store and the latest settings found (None if there are none).
    pub fn open(flash: &mut dyn Flash, region: Region) -> Result<(Self, Option<Settings>), FlashError> {
        let region = Region { offset: region.offset, size: region.size / SECTOR_BYTES * SECTOR_BYTES };
        let mut store = Self { region, latest: None };
        let mut found = None;
        let mut record = [0u8; RECORD_BYTES as usize];
        for slot in 0..store.slot_count() {
            flash.read(store.slot_offset(slot), &mut record)?;
            if let Some((sequence, settings)) = decode(&record) {
                let newer = match store.latest {
                    None => true,
                    Some((_, latest)) => sequence.wrapping_sub(latest) as i32 > 0,
                };
                if newer {
                    store.latest = Some((slot, sequence));
                    found = Some(settings);
                }
            }
        }
        Ok((store, found))
    }

    fn slot_count(&self) -> u32 {
        self.region.size / RECORD_BYTES
    }

    fn slot_offset(&self, slot: u32) -> u32 {
        self.region.offset + slot * RECORD_BYTES
    }

    fn erase_sector_of(&self, flash: &mut dyn Flash, slot: u32) -> Result<(), FlashError> {
        let start = self.region.offset + slot / SLOTS_PER_SECTOR * SECTOR_BYTES;
        flash.erase(start, start + SECTOR_BYTES)
    }

    pub fn save(&mut self, flash: &mut dyn Flash, settings: &Settings) -> Result<(), FlashError> {
        if self.slot_count() == 0 {
            return Err(FlashError);
        }
        let (mut slot, sequence) = match self.latest {
            Some((slot, sequence)) => ((slot + 1) % self.slot_count(), sequence.wrapping_add(1)),
            None => (0, 1),
        };
        if slot % SLOTS_PER_SECTOR == 0 {
            self.erase_sector_of(flash, slot)?;
        } else {
            // The next slot should be erased; if not (e.g. power lost mid-write), erase the whole
            // sector and write from its start. The latest record goes with it, but this one replaces
            // it right away.
            let mut existing = [0u8; RECORD_BYTES as usize];
            flash.read(self.slot_offset(slot), &mut existing)?;
            if existing.iter().any(|&byte| byte != 0xFF) {
                self.erase_sector_of(flash, slot)?;
                slot -= slot % SLOTS_PER_SECTOR;
            }
        }
        flash.write(self.slot_offset(slot), &encode(settings, sequence))?;
        self.latest = Some((slot, sequence));
        Ok(())
    }
}

/// On a box shared with Muse, the otadata partition and the number of the slot this firmware
/// isn't running from (`running` is the running app's flash offset), if that slot holds an app.
pub fn other_app(flash: &mut dyn Flash, running: u32) -> Result<Option<(Region, u32)>, FlashError> {
    let (Some(otadata), Some(ota_0), Some(ota_1)) =
        (find_partition(flash, "otadata"), find_partition(flash, "ota_0"), find_partition(flash, "ota_1"))
    else {
        return Ok(None);
    };
    let (target, slot) = if running == ota_0.offset { (ota_1, 1) } else { (ota_0, 0) };
    // An app image starts with the 0xE9 magic byte; an empty slot reads 0xFF.
    let mut magic = [0u8; 4];
    flash.read(target.offset, &mut magic)?;
    Ok((magic[0] == 0xE9).then_some((otadata, slot)))
}

/// On a box shared with Muse (ota_0 and ota_1), makes the bootloader start the slot this firmware
/// isn't running from: `running` is the flash offset of the running app. Returns false, writing
/// nothing, on a single-firmware layout or when the other slot holds no app image.
///
/// otadata is written the way ESP-IDF's esp_ota_set_boot_partition does it, since its bootloader
/// reads it: two 32-byte entries, one per 4 KB sector, each `seq, 20-byte label, state, crc`; the
/// valid entry with the highest seq wins and boots ota_((seq - 1) % 2). esp-bootloader-esp-idf
/// 0.6 can't be used for this: from blank otadata it selects ota_0 when asked for the next slot,
/// and writes seq 0, which is invalid, when asked for ota_1.
pub fn boot_other_app(flash: &mut dyn Flash, running: u32) -> Result<bool, FlashError> {
    let Some((otadata, slot)) = other_app(flash, running)? else {
        return Ok(false);
    };

    const ENTRY_BYTES: usize = 32;
    let mut active: Option<(usize, u32)> = None;
    for sector in 0..2 {
        let mut entry = [0u8; ENTRY_BYTES];
        flash.read(otadata.offset + sector as u32 * SECTOR_BYTES, &mut entry)?;
        let word = |at: usize| u32::from_le_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]]);
        let (seq, state, crc) = (word(0), word(24), word(28));
        // ESP_OTA_IMG_INVALID and ESP_OTA_IMG_ABORTED entries don't count.
        let valid = seq != u32::MAX && state != 3 && state != 4 && crc == crc32(u32::MAX, &seq.to_le_bytes());
        if valid && active.is_none_or(|(_, best)| seq > best) {
            active = Some((sector, seq));
        }
    }
    let mut seq = active.map_or(0, |(_, seq)| seq) + 1;
    while (seq - 1) % 2 != slot {
        seq += 1;
    }
    let sector = active.map_or(0, |(sector, _)| 1 - sector);
    let mut entry = [0xFFu8; ENTRY_BYTES];
    entry[0..4].copy_from_slice(&seq.to_le_bytes());
    // State stays 0xFFFFFFFF (ESP_OTA_IMG_UNDEFINED): the bootloader doesn't do rollback.
    entry[28..32].copy_from_slice(&crc32(u32::MAX, &seq.to_le_bytes()).to_le_bytes());
    let at = otadata.offset + sector as u32 * SECTOR_BYTES;
    flash.erase(at, at + SECTOR_BYTES)?;
    flash.write(at, &entry)?;
    Ok(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// NOR flash in memory: writes can only turn 1s into 0s, erases reset the range to 0xFF.
    pub(crate) struct MemoryFlash {
        pub bytes: Vec<u8>,
        pub erases: Vec<(u32, u32)>,
    }

    impl MemoryFlash {
        pub(crate) fn new(size: usize) -> Self {
            Self { bytes: vec![0xFF; size], erases: Vec::new() }
        }
    }

    impl Flash for MemoryFlash {
        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
            let start = offset as usize;
            let source = self.bytes.get(start..start + bytes.len()).ok_or(FlashError)?;
            bytes.copy_from_slice(source);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
            assert_eq!(offset % 4, 0, "write offset not aligned");
            assert_eq!(bytes.len() % 4, 0, "write length not aligned");
            let start = offset as usize;
            let target = self.bytes.get_mut(start..start + bytes.len()).ok_or(FlashError)?;
            for (cell, &byte) in target.iter_mut().zip(bytes) {
                *cell &= byte;
            }
            Ok(())
        }

        fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
            assert_eq!(from % SECTOR_BYTES, 0);
            assert_eq!(to % SECTOR_BYTES, 0);
            self.bytes.get_mut(from as usize..to as usize).ok_or(FlashError)?.fill(0xFF);
            self.erases.push((from, to));
            Ok(())
        }
    }

    const NVS: Region = Region { offset: 0x9000, size: 0x6000 };

    fn settings(completed: u32) -> Settings {
        Settings { tally: Tally { day: 20260926, completed, focus_s: completed * 1500 }, volume: 65 }
    }

    #[test]
    fn an_empty_region_has_no_settings() {
        let mut flash = MemoryFlash::new(0x10000);
        let (_, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn old_nvs_pages_are_not_mistaken_for_settings() {
        let mut flash = MemoryFlash::new(0x10000);
        for (index, byte) in flash.bytes[0x9000..0xF000].iter_mut().enumerate() {
            *byte = (index * 31 % 251) as u8;
        }
        let (mut store, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, None);
        store.save(&mut flash, &settings(1)).unwrap();
        assert_eq!(flash.erases, [(0x9000, 0xA000)]);
        let (_, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, Some(settings(1)));
    }

    #[test]
    fn the_latest_record_wins_across_sectors_and_wraparound() {
        let mut flash = MemoryFlash::new(0x10000);
        let (mut store, _) = SettingsStore::open(&mut flash, NVS).unwrap();
        // Six sectors hold 768 slots; write a bit more than two rounds.
        for completed in 0..1700 {
            store.save(&mut flash, &settings(completed)).unwrap();
        }
        let (_, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, Some(settings(1699)));
        // Only one erase per 128 records.
        assert_eq!(flash.erases.len(), 1700usize.div_ceil(128));
    }

    #[test]
    fn a_dirty_slot_is_recovered_by_erasing_its_sector() {
        let mut flash = MemoryFlash::new(0x10000);
        let (mut store, _) = SettingsStore::open(&mut flash, NVS).unwrap();
        store.save(&mut flash, &settings(1)).unwrap();
        // The next slot holds half a record.
        flash.bytes[0x9000 + 32] = 0x12;
        store.save(&mut flash, &settings(2)).unwrap();
        let (_, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, Some(settings(2)));
    }

    #[test]
    fn partitions_are_found_by_label() {
        let mut flash = MemoryFlash::new(0x10000);
        let mut entry = |index: usize, label: &str, offset: u32, size: u32| {
            let at = 0x8000 + index * 32;
            let bytes = &mut flash.bytes[at..at + 32];
            bytes.fill(0);
            bytes[0] = 0xAA;
            bytes[1] = 0x50;
            bytes[4..8].copy_from_slice(&offset.to_le_bytes());
            bytes[8..12].copy_from_slice(&size.to_le_bytes());
            bytes[12..12 + label.len()].copy_from_slice(label.as_bytes());
        };
        entry(0, "nvs", 0x9000, 0x6000);
        entry(1, "phy_init", 0xF000, 0x1000);
        entry(2, "factory", 0x10000, 0x400000);
        entry(3, "voices", 0x410000, 0x200000);
        assert_eq!(find_partition(&mut flash, "voices"), Some(Region { offset: 0x410000, size: 0x200000 }));
        assert_eq!(find_partition(&mut flash, "nvs"), Some(NVS));
        assert_eq!(find_partition(&mut flash, "missing"), None);
    }

    /// The layout of a box shared with Muse, with an app image in ota_0 and, if `muse`, in ota_1.
    fn shared_box(muse: bool) -> MemoryFlash {
        let mut flash = MemoryFlash::new(0x430000);
        let entries = [("otadata", 0x15000u32, 0x2000u32), ("ota_0", 0x20000, 0x400000), ("ota_1", 0x420000, 0x400000)];
        for (index, (label, offset, size)) in entries.iter().enumerate() {
            let at = 0x8000 + index * 32;
            let bytes = &mut flash.bytes[at..at + 32];
            bytes.fill(0);
            bytes[0] = 0xAA;
            bytes[1] = 0x50;
            bytes[4..8].copy_from_slice(&offset.to_le_bytes());
            bytes[8..12].copy_from_slice(&size.to_le_bytes());
            bytes[12..12 + label.len()].copy_from_slice(label.as_bytes());
        }
        flash.bytes[0x20000] = 0xE9;
        if muse {
            flash.bytes[0x420000] = 0xE9;
        }
        flash
    }

    /// The slot ESP-IDF's bootloader would boot, as its bootloader_utility_get_selected_boot_partition does.
    fn booted_slot(flash: &MemoryFlash) -> Option<u32> {
        let entry = |at: usize| {
            let word = |i: usize| u32::from_le_bytes(flash.bytes[at + i..at + i + 4].try_into().unwrap());
            let seq = word(0);
            (seq != u32::MAX && word(28) == crc32(u32::MAX, &seq.to_le_bytes())).then_some(seq)
        };
        entry(0x15000).max(entry(0x16000)).map(|seq| (seq - 1) % 2)
    }

    #[test]
    fn switching_from_blank_otadata_boots_muse_then_back() {
        let mut flash = shared_box(true);
        assert_eq!(booted_slot(&flash), None, "blank otadata boots ota_0");
        assert_eq!(boot_other_app(&mut flash, 0x20000), Ok(true));
        assert_eq!(booted_slot(&flash), Some(1));
        // The CRC is ESP-IDF's: the bootloader accepts seq 1 with crc 0x4743989a.
        assert_eq!(crc32(u32::MAX, &1u32.to_le_bytes()), 0x4743_989A);
        assert_eq!(boot_other_app(&mut flash, 0x420000), Ok(true));
        assert_eq!(booted_slot(&flash), Some(0));
        assert_eq!(boot_other_app(&mut flash, 0x20000), Ok(true));
        assert_eq!(booted_slot(&flash), Some(1));
    }

    #[test]
    fn otadata_left_by_esp_idf_is_carried_on() {
        // What the box held after the 0.6 crate's attempt: seq 1 (ota_0) in the first sector.
        let mut flash = shared_box(true);
        flash.bytes[0x15000..0x15004].copy_from_slice(&1u32.to_le_bytes());
        flash.bytes[0x15018..0x1501C].copy_from_slice(&2u32.to_le_bytes());
        flash.bytes[0x1501C..0x15020].copy_from_slice(&0x4743_989Au32.to_le_bytes());
        assert_eq!(boot_other_app(&mut flash, 0x20000), Ok(true));
        assert_eq!(booted_slot(&flash), Some(1));
        assert_eq!(&flash.bytes[0x16000..0x16004], &2u32.to_le_bytes(), "the next seq, in the other sector");
    }

    #[test]
    fn no_switch_without_a_second_app() {
        let mut flash = shared_box(false);
        assert_eq!(boot_other_app(&mut flash, 0x20000), Ok(false));
        assert_eq!(booted_slot(&flash), None);
        let mut single = MemoryFlash::new(0x10000);
        assert_eq!(boot_other_app(&mut single, 0x10000), Ok(false));
    }
}
