//! On-chip flash, shared by the main loop (settings, voice pack writes) and the audio task (reading
//! lines chunk by chunk). Both take turns on the same executor; the critical section only makes the
//! borrowing explicit.

use core::cell::RefCell;

use critical_section::Mutex;
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN;
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

/// Marks the running image as working, if it was just installed over the air and is waiting for that
/// (ADR-0013); otherwise the bootloader rolls back to the previous slot on the next boot. Does nothing
/// after a USB flash, which leaves otadata blank, or on the single-slot layout, which has no otadata.
/// The write erases one otadata sector with interrupts off, so it runs at boot, before the Mac talks.
pub fn confirm_running_image() {
    critical_section::with(|cs| {
        let mut flash = FLASH.borrow_ref_mut(cs);
        let Some(flash) = flash.as_mut() else { return };
        let mut table = [0u8; PARTITION_TABLE_MAX_LEN];
        let Ok(mut ota) = OtaUpdater::new(flash, &mut table) else { return };
        if matches!(ota.current_ota_state(), Ok(OtaImageState::New | OtaImageState::PendingVerify)) {
            // If this fails the image is rolled back on the next boot, which is the safe direction.
            let _ = ota.set_current_ota_state(OtaImageState::Valid);
        }
    });
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
