//! 片上 flash：分区表、设置记录。
//!
//! C 固件用 ESP-IDF 的 NVS 存当日番茄记录与音量。esp-hal 没有 NVS，这里在
//! 原来的 `nvs` 分区上写一个追加式的小日志：每条记录 32 字节、自带序号与
//! CRC，读的时候取序号最大的有效一条。一天只写几次，追加写把擦除摊到每
//! 128 条一次，不磨损 flash。第一次开机读不到有效记录（那里还是 NVS 的旧页），
//! 就当全新设备，从第一个扇区擦起。

use crate::pomodoro::Tally;
use crate::voice_pack::crc32;

pub const SECTOR_BYTES: u32 = 4096;

/// flash 操作失败。ESP-IDF 那边的名字是 ESP_ERR_FLASH_OP_FAIL，回执沿用它。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlashError;

/// 设备的 flash。偏移都是整片 flash 的绝对地址；写入的偏移与长度必须是
/// 4 的倍数，擦除按扇区对齐。
pub trait Flash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError>;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError>;
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError>;
}

/// flash 上的一段区域。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub offset: u32,
    pub size: u32,
}

pub const PARTITION_TABLE_OFFSET: u32 = 0x8000;
const PARTITION_ENTRY_BYTES: usize = 32;
const PARTITION_MAX_ENTRIES: usize = 95;

/// 按标签找分区。ESP-IDF 分区表：每项 32 字节，魔数 0xAA 0x50，之后是类型、
/// 子类型、u32 偏移、u32 大小、16 字节标签、u32 标志；以全 0xFF 或 MD5 项结束。
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

/// 设备自己记着的事：当日番茄记录与音量。
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
    /// 最近一条有效记录所在的槽位与它的序号。
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
    /// 扫一遍区域，返回存储本身与读到的最新设置（没有就是 None）。
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
            // 下一格本该是擦过的；不是（掉电写了半条之类），就把整个扇区擦掉
            // 从扇区开头写。最新那条会跟着被擦，但马上就被这一条取代。
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// 内存里的 NOR flash：写只能把 1 变 0，擦除把整段变回 0xFF。
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
            assert_eq!(offset % 4, 0, "写入偏移没对齐");
            assert_eq!(bytes.len() % 4, 0, "写入长度没对齐");
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
        // 六个扇区 768 格，写两圈多一点。
        for completed in 0..1700 {
            store.save(&mut flash, &settings(completed)).unwrap();
        }
        let (_, found) = SettingsStore::open(&mut flash, NVS).unwrap();
        assert_eq!(found, Some(settings(1699)));
        // 每 128 条才擦一次。
        assert_eq!(flash.erases.len(), 1700usize.div_ceil(128));
    }

    #[test]
    fn a_dirty_slot_is_recovered_by_erasing_its_sector() {
        let mut flash = MemoryFlash::new(0x10000);
        let (mut store, _) = SettingsStore::open(&mut flash, NVS).unwrap();
        store.save(&mut flash, &settings(1)).unwrap();
        // 下一格被写了半条。
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
}
