//! Microphone capture for the bring-up diagnostic (`device.mic.record`, firmware-core's `mic`): I2S0's
//! receiver, clocked by the transmitter's BCLK and WS (signal loopback, as ESP-IDF does for full duplex),
//! reads GPIO47 into a DMA ring only while a capture is asked for, and the samples are copied into PSRAM
//! until there are enough.
//!
//! The ring is taken from the internal heap for the capture and given back after, not a static: every
//! static in internal RAM comes out of the main task's stack, which is whatever DRAM the statics leave
//! (LESSONS.md). A 24 KB static ring cost the stack 24 KB and the box boot-looped on a stack overflow.

use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Timer};
use esp_alloc::{HEAP, MemoryCapability};
use esp_hal::Async;
use esp_hal::dma::aligned::DmaAlignedMut;
use esp_hal::dma::{DmaDescriptor, DmaRxStreamBuf};
use esp_hal::i2s::master::I2sRx;

/// 24 kHz × stereo × 16 bit = 96 KB/s, so 12 KB is 125 ms: how long the main loop may hold the executor
/// before samples are lost (the capture then ends with `STALLED`).
const RING_BYTES: usize = 12000;
/// esp-hal's stream buffer wants at least four descriptors; 3000 bytes each.
const RING_DESCRIPTORS: usize = 4;

/// No sample for this long means the DMA stopped (its ring overflowed) or the clock isn't running.
const STALL_MS: u64 = 500;

/// How many bytes to capture.
pub static START: Signal<CriticalSectionRawMutex, usize> = Signal::new();
/// The capture, or why it stopped short.
pub static DONE: Signal<CriticalSectionRawMutex, Result<Vec<u8>, &'static str>> = Signal::new();

/// The DMA ring and its descriptors, in internal RAM (the stream buffer doesn't keep PSRAM coherent).
struct Ring {
    descriptors: *mut DmaDescriptor,
    bytes: *mut u8,
}

impl Ring {
    fn descriptors_layout() -> Layout {
        Layout::array::<DmaDescriptor>(RING_DESCRIPTORS).unwrap()
    }

    fn bytes_layout() -> Layout {
        Layout::from_size_align(RING_BYTES, 4).unwrap()
    }

    fn new() -> Option<Self> {
        // SAFETY: both layouts are non-zero, and each block is initialized before any reference is made to it.
        unsafe {
            let descriptors = HEAP.alloc_caps(MemoryCapability::Internal.into(), Self::descriptors_layout()) as *mut DmaDescriptor;
            let bytes = HEAP.alloc_caps(MemoryCapability::Internal.into(), Self::bytes_layout());
            let ring = Ring { descriptors, bytes };
            if descriptors.is_null() || bytes.is_null() {
                return None;
            }
            for index in 0..RING_DESCRIPTORS {
                descriptors.add(index).write(DmaDescriptor::EMPTY);
            }
            bytes.write_bytes(0, RING_BYTES);
            Some(ring)
        }
    }

    /// SAFETY: the buffer made here must be dropped before the ring is.
    unsafe fn buffer(&mut self) -> Option<DmaRxStreamBuf> {
        // SAFETY: the blocks are ours alone and outlive the buffer (the caller's promise).
        let (descriptors, bytes) = unsafe {
            (core::slice::from_raw_parts_mut(self.descriptors, RING_DESCRIPTORS), core::slice::from_raw_parts_mut(self.bytes, RING_BYTES))
        };
        DmaRxStreamBuf::new(DmaAlignedMut::new(descriptors).ok()?, DmaAlignedMut::new(bytes).ok()?).ok()
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: allocated by `new` with these layouts; dealloc ignores null.
        unsafe {
            if !self.descriptors.is_null() {
                HEAP.dealloc(self.descriptors as *mut u8, Self::descriptors_layout());
            }
            if !self.bytes.is_null() {
                HEAP.dealloc(self.bytes, Self::bytes_layout());
            }
        }
    }
}

#[embassy_executor::task]
pub async fn mic_task(mut rx: I2sRx<'static, Async>) {
    loop {
        let wanted = START.wait().await;
        let (result, idle) = capture(rx, wanted).await;
        rx = idle;
        DONE.signal(result);
    }
}

async fn capture(rx: I2sRx<'static, Async>, wanted: usize) -> (Result<Vec<u8>, &'static str>, I2sRx<'static, Async>) {
    // The capture lives on the heap, which is PSRAM first (main.rs).
    let mut capture = Vec::new();
    if capture.try_reserve_exact(wanted).is_err() {
        return (Err("MEMORY"), rx);
    }
    let Some(mut ring) = Ring::new() else { return (Err("MEMORY"), rx) };
    // SAFETY: the buffer is dropped below, before `ring`.
    let Some(buffer) = (unsafe { ring.buffer() }) else { return (Err("MEMORY"), rx) };
    let mut transfer = match rx.read(buffer) {
        Ok(transfer) => transfer,
        Err((_, rx, buffer)) => {
            drop(buffer);
            return (Err("I2S"), rx);
        }
    };
    let mut last_data = Instant::now();
    let result = loop {
        while capture.len() < wanted {
            let available = transfer.peek();
            if available.is_empty() {
                break;
            }
            let take = available.len().min(wanted - capture.len());
            capture.extend_from_slice(&available[..take]);
            transfer.consume(take);
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
    let (rx, buffer) = transfer.stop();
    drop(buffer);
    drop(ring);
    (result, rx)
}
