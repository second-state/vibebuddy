//! 片上 flash：主循环（设置、语音包写入）与音频任务（按块读语音）共用。
//! 两者在同一个执行器上轮流跑，临界区只是把借用关系交代清楚。

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

/// 读任意偏移、任意长度：按 4 字节对齐读进一小块缓冲再挑出要的部分。
/// 语音每一句的偏移与长度不保证 4 字节对齐。
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

/// esp-storage 对没有按字对齐的缓冲会在栈上垫一个 4 KB 的扇区缓冲。
#[repr(align(4))]
struct Aligned([u8; 260]);

/// 给 [`vibebuddy_firmware_core::firmware::Board`] 的 flash 口。
pub struct SharedFlash;

impl Flash for SharedFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        with_flash(|flash| flash.read_nor(offset, bytes))
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        with_flash(|flash| flash.write_nor(offset, bytes))
    }

    /// 按 64 KB 一段擦，每段单独进出临界区：擦整个 voices 分区要好几秒，
    /// 一直关着中断，UART 的接收中断就跑不了，128 字节的 FIFO 会溢出。
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
