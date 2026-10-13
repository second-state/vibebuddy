//! Microphone capture for the bring-up diagnostic (`device.mic.record`, firmware-core's `mic`): I2S0's
//! receiver, clocked by the transmitter's BCLK and WS (signal loopback, as ESP-IDF does for full duplex),
//! reads GPIO47 by DMA straight into one linear buffer in PSRAM, as long as the whole capture.
//!
//! Nothing here is a static in internal RAM beyond the task itself: every such static comes out of the
//! main task's stack (LESSONS.md). The descriptors (12 bytes per 4 KB) come from the internal heap for the
//! capture and go back after.
//!
//! The first build streamed through a 12 KB ring and on the box never delivered a sample (`STALLED`), with
//! nothing saying why. So each capture also reports `MIC DEBUG` lines: the I2S and DMA registers and the
//! level changes seen on the pins 100 ms in and at the end, which the next flash can be read against.

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::fmt::Write;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Timer};
use esp_alloc::{HEAP, MemoryCapability};
use esp_hal::Async;
use esp_hal::dma::aligned::DmaAlignedMut;
use esp_hal::dma::{DmaDescriptor, DmaRxBuf};
use esp_hal::i2s::master::I2sRx;

/// PSRAM receive buffers must be aligned to the external burst size (16 bytes by default); 64 covers any.
const ALIGN: usize = 64;
/// Bytes one descriptor covers at least (esp-hal uses 4096 minus the burst alignment); spares go unused.
const DESCRIPTOR_BYTES: usize = 4096 - ALIGN;
/// The capture takes its own length plus this long before it counts as stalled.
const SLACK_MS: u64 = 1000;

/// How many bytes to capture.
pub static START: Signal<CriticalSectionRawMutex, usize> = Signal::new();
/// The capture, or why it stopped short.
pub static DONE: Signal<CriticalSectionRawMutex, Result<Vec<u8>, &'static str>> = Signal::new();
/// `MIC DEBUG` lines for the main loop to print.
pub static NOTES: Channel<CriticalSectionRawMutex, String, 4> = Channel::new();

/// A block from the heap with the given capabilities, freed on drop.
struct Block {
    pointer: *mut u8,
    layout: Layout,
}

impl Block {
    fn new(capability: MemoryCapability, size: usize) -> Option<Self> {
        let layout = Layout::from_size_align(size, ALIGN).ok()?;
        // SAFETY: size is non-zero (callers ask for at least one descriptor or frame).
        let pointer = unsafe { HEAP.alloc_caps(capability.into(), layout) };
        if pointer.is_null() {
            return None;
        }
        // SAFETY: just allocated with this size; zeroed before any reference to it is made.
        unsafe { pointer.write_bytes(0, size) };
        Some(Self { pointer, layout })
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { HEAP.dealloc(self.pointer, self.layout) };
    }
}

/// Reads a 32-bit peripheral register.
fn register(address: u32) -> u32 {
    // SAFETY: a read of a fixed ESP32-S3 peripheral address, with no side effects for these registers.
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

const I2S0: u32 = 0x6000_F000;
/// GDMA channel 1 (I2S0's DMA_CH1 in main.rs); channels are 0xC0 apart.
const GDMA_CH1: u32 = 0x6003_F000 + 0xC0;
const GPIO_IN: u32 = 0x6000_403C;
const GPIO_IN1: u32 = 0x6000_4040;

/// How often GPIO47 (DIN), GPIO21 (BCLK) and GPIO13 (WS) changed level over 2000 quick reads. The two
/// clocks are outputs and may read 0 if their input path is off; DIN is an input and reads true.
fn pin_activity() -> [u32; 3] {
    let sample = || [register(GPIO_IN1) >> 15 & 1, register(GPIO_IN) >> 21 & 1, register(GPIO_IN) >> 13 & 1];
    let mut last = sample();
    let mut changes = [0u32; 3];
    for _ in 0..2000 {
        let now = sample();
        for pin in 0..3 {
            changes[pin] += (now[pin] != last[pin]) as u32;
        }
        last = now;
    }
    changes
}

/// One line of the receive path's state: descriptors done so far, then I2S0 RX_CONF, TX_CONF, INT_RAW,
/// RX_CLKM_CONF, RXEOF_NUM, TX state, and GDMA channel 1's IN_INT_RAW, INFIFO_STATUS, IN_LINK, IN_STATE,
/// IN_DSCR, IN_PERI_SEL, OUT_STATE, OUT_PERI_SEL, then the pin level changes.
fn note(stage: &str, descriptors: *const DmaDescriptor, count: usize) {
    // SAFETY: the descriptors live for the capture; the DMA may write them, so read them volatile.
    let done = (0..count).filter(|&index| unsafe { !core::ptr::read_volatile(descriptors.add(index)).flags.owner() }).count();
    let [din, bclk, ws] = pin_activity();
    let mut line = String::new();
    let _ = write!(
        line,
        "MIC DEBUG {stage} DESC {done}/{count} I2S RXCONF {:08X} TXCONF {:08X} INTRAW {:08X} RXCLKM {:08X} RXEOF {:08X} TXSTATE {:08X} \
         DMA ININT {:08X} INFIFO {:08X} INLINK {:08X} INSTATE {:08X} INDSCR {:08X} INPERI {:08X} OUTSTATE {:08X} OUTPERI {:08X} \
         PINS DIN {din} BCLK {bclk} WS {ws}",
        register(I2S0 + 0x20),
        register(I2S0 + 0x24),
        register(I2S0 + 0x0C),
        register(I2S0 + 0x30),
        register(I2S0 + 0x64),
        register(I2S0 + 0x6C),
        register(GDMA_CH1 + 0x08),
        register(GDMA_CH1 + 0x18),
        register(GDMA_CH1 + 0x20),
        register(GDMA_CH1 + 0x24),
        register(GDMA_CH1 + 0x30),
        register(GDMA_CH1 + 0x48),
        register(GDMA_CH1 + 0x84),
        register(GDMA_CH1 + 0xA8),
    );
    let _ = NOTES.try_send(line);
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
    let size = wanted.next_multiple_of(ALIGN);
    let count = size.div_ceil(DESCRIPTOR_BYTES);
    let (Some(data), Some(descriptors)) = (Block::new(MemoryCapability::External, size), Block::new(MemoryCapability::Internal, count * size_of::<DmaDescriptor>()))
    else {
        return (Err("MEMORY"), rx);
    };
    let first = descriptors.pointer as *mut DmaDescriptor;
    // SAFETY: both blocks are ours alone, zeroed (a zeroed descriptor is `DmaDescriptor::EMPTY`), and outlive
    // the buffer, which is dropped below before them.
    let buffer = unsafe {
        let descriptor_slice = core::slice::from_raw_parts_mut(first, count);
        let data_slice = core::slice::from_raw_parts_mut(data.pointer, size);
        match (DmaAlignedMut::new(descriptor_slice), DmaAlignedMut::new(data_slice)) {
            (Ok(descriptor_slice), Ok(data_slice)) => DmaRxBuf::new(descriptor_slice, data_slice).ok(),
            _ => None,
        }
    };
    let Some(buffer) = buffer else { return (Err("DMA BUFFER"), rx) };
    // The descriptors esp-hal linked, and the last of them: when the DMA hands that one back, it's full.
    let (mut linked, mut last) = (1, first);
    // SAFETY: esp-hal just linked these within the block; the DMA hasn't started.
    while let Some(next) = unsafe { (*last).next.as_mut() } {
        last = next as *mut DmaDescriptor;
        linked += 1;
    }
    let transfer = match rx.read(buffer) {
        Ok(transfer) => transfer,
        Err((_, rx, buffer)) => {
            drop(buffer);
            return (Err("I2S"), rx);
        }
    };

    let started = Instant::now();
    let deadline = (wanted / 96) as u64 + SLACK_MS;
    let mut noted = false;
    let finished = loop {
        Timer::after_millis(20).await;
        if !noted && started.elapsed().as_millis() >= 100 {
            note("T100", first, linked);
            noted = true;
        }
        // SAFETY: as in `note`.
        if unsafe { !core::ptr::read_volatile(last).flags.owner() } {
            break true;
        }
        if started.elapsed().as_millis() > deadline {
            break false;
        }
    };
    note(if finished { "END" } else { "TIMEOUT" }, first, linked);
    let (rx, buffer) = transfer.stop();
    let result = if finished {
        let mut capture = Vec::new();
        if capture.try_reserve_exact(wanted).is_ok() {
            // The buffer's own read invalidates the PSRAM cache for what the DMA wrote.
            let mut received = 0;
            for piece in buffer.received_data() {
                let take = piece.len().min(wanted - received);
                capture.extend_from_slice(&piece[..take]);
                received += take;
            }
            if capture.len() == wanted { Ok(capture) } else { Err("SHORT") }
        } else {
            Err("MEMORY")
        }
    } else {
        Err("STALLED")
    };
    drop(buffer);
    drop(descriptors);
    drop(data);
    (result, rx)
}
