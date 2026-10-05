//! On-chip flash, shared by the main loop (settings, voice pack writes) and the audio task (reading
//! lines chunk by chunk). Both take turns on the same executor; the critical section only makes the
//! borrowing explicit.

use core::cell::RefCell;

use critical_section::Mutex;
use esp_storage::FlashStorage;
use vibebuddy_firmware_core::storage::{Flash, FlashError};

static FLASH: Mutex<RefCell<Option<FlashStorage<'static>>>> = Mutex::new(RefCell::new(None));

pub fn install(flash: FlashStorage<'static>) {
    critical_section::with(|cs| FLASH.borrow_ref_mut(cs).replace(flash));
}

fn with_flash<T>(action: impl FnOnce(&mut FlashStorage<'static>) -> Result<T, esp_storage::FlashStorageError>) -> Result<T, FlashError> {
    critical_section::with(|cs| match FLASH.borrow_ref_mut(cs).as_mut() {
        Some(flash) => action(flash).map_err(|_| FlashError),
        None => Err(FlashError),
    })
}

/// Reads any offset and length: reads 4-byte aligned into a small buffer, then picks out the wanted
/// part. Voice line offsets and lengths are not guaranteed to be 4-byte aligned.
pub fn read_unaligned(offset: u32, out: &mut [u8]) -> Result<(), FlashError> {
    let mut window = Aligned([0u8; 260]);
    let mut done = 0;
    while done < out.len() {
        let at = offset + done as u32;
        let start = at & !3;
        let skip = (at - start) as usize;
        let take = (out.len() - done).min(256 - skip);
        let length = (skip + take).div_ceil(4) * 4;
        with_flash(|flash| flash.read_nor(start, &mut window.0[..length]))?;
        out[done..done + take].copy_from_slice(&window.0[skip..skip + take]);
        done += take;
    }
    Ok(())
}

/// For a buffer that is not word-aligned, esp-storage puts a 4 KB sector buffer on the stack.
#[repr(align(4))]
struct Aligned([u8; 260]);

/// The flash port for [`vibebuddy_firmware_core::firmware::Board`].
pub struct SharedFlash;

impl Flash for SharedFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        with_flash(|flash| flash.read_nor(offset, bytes))
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        with_flash(|flash| flash.write_nor(offset, bytes))
    }

    /// Erases in 64 KB steps, each in its own critical section: erasing the whole voices partition
    /// takes several seconds, and with interrupts off all that time the UART receive interrupt cannot
    /// run and the 128-byte FIFO overflows.
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        const STEP: u32 = 64 * 1024;
        let mut at = from;
        while at < to {
            let end = (at + STEP).min(to);
            with_flash(|flash| flash.erase(at, end))?;
            at = end;
        }
        Ok(())
    }
}

/// The flash offset of the app the bootloader started, read from the MMU.
pub fn running_app_offset() -> Option<u32> {
    use esp_bootloader_esp_idf::partitions::{PARTITION_TABLE_MAX_LEN, read_partition_table};
    critical_section::with(|cs| {
        let mut slot = FLASH.borrow_ref_mut(cs);
        let flash = slot.as_mut()?;
        let mut table = [0u8; PARTITION_TABLE_MAX_LEN];
        let table = read_partition_table(flash, &mut table).ok()?;
        table.booted_partition().ok().flatten().map(|entry| entry.offset())
    })
}
