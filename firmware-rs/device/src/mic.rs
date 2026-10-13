//! Microphone capture for the bring-up diagnostic (`device.mic.record`, firmware-core's `mic`): I2S0's
//! receiver, clocked by the transmitter's BCLK and WS (signal loopback, as ESP-IDF does for full duplex),
//! reads GPIO47 into a DMA stream buffer only while a capture is asked for, and the samples are copied
//! into PSRAM until there are enough.

use alloc::vec::Vec;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Timer};
use esp_hal::Async;
use esp_hal::dma::DmaRxStreamBuf;
use esp_hal::i2s::master::I2sRx;

/// 24 kHz × stereo × 16 bit = 96 KB/s, so 24 KB is 250 ms: how long the main loop may hold the executor
/// before samples are lost.
pub const STREAM_BYTES: usize = 24000;
pub const STREAM_CHUNK: usize = 2400;

/// No sample for this long means the DMA stopped (its buffer overflowed) or the clock isn't running.
const STALL_MS: u64 = 500;

/// How many bytes to capture.
pub static START: Signal<CriticalSectionRawMutex, usize> = Signal::new();
/// The capture, or why it stopped short.
pub static DONE: Signal<CriticalSectionRawMutex, Result<Vec<u8>, &'static str>> = Signal::new();

#[embassy_executor::task]
pub async fn mic_task(rx: I2sRx<'static, Async>, buffer: DmaRxStreamBuf) {
    let mut idle = Some((rx, buffer));
    loop {
        let wanted = START.wait().await;
        let (rx, buffer) = idle.take().unwrap();
        // The capture lives on the heap, which is PSRAM first (main.rs).
        let mut capture = Vec::new();
        if capture.try_reserve_exact(wanted).is_err() {
            idle = Some((rx, buffer));
            DONE.signal(Err("MEMORY"));
            continue;
        }
        let mut transfer = match rx.read(buffer) {
            Ok(transfer) => transfer,
            Err((_, rx, buffer)) => {
                idle = Some((rx, buffer));
                DONE.signal(Err("I2S"));
                continue;
            }
        };
        let mut last_data = Instant::now();
        let mut chunk = [0u8; STREAM_CHUNK];
        let result = loop {
            while capture.len() < wanted {
                let count = transfer.pop(&mut chunk[..STREAM_CHUNK.min(wanted - capture.len())]);
                if count == 0 {
                    break;
                }
                capture.extend_from_slice(&chunk[..count]);
                last_data = Instant::now();
            }
            if capture.len() >= wanted {
                break Ok(capture);
            }
            if last_data.elapsed().as_millis() > STALL_MS {
                break Err("STALLED");
            }
            Timer::after_millis(5).await;
        };
        idle = Some(transfer.stop());
        DONE.signal(result);
    }
}
