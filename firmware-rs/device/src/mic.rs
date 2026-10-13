//! Microphone capture for the bring-up diagnostic (`device.mic.record`, firmware-core's `mic`): I2S0's
//! receiver, clocked by the transmitter's BCLK and WS (signal loopback, as ESP-IDF does for full duplex),
//! reads GPIO47 by DMA straight into one linear buffer in PSRAM, as long as the whole capture.
//!
//! Nothing here is a static in internal RAM beyond the task itself: every such static comes out of the
//! main task's stack (LESSONS.md). The descriptors (12 bytes per 4 KB) come from the internal heap for the
//! capture and go back after.
//!
//! The I2S receiver ends a DMA descriptor early at every EOF it signals. esp-hal's `rx_start` writes
//! RXEOF_NUM = 0xFFE (its `usize::MAX - 1` cut to the 12-bit field), and on the box the EOFs came every
//! 4094 bytes: descriptor lengths 4080, 14, 4080, 14, … (2026-10-13). So the ESP32-S3 counts RXEOF_NUM
//! in bytes, as ESP-IDF's driver uses it (it writes the DMA buffer size in bytes), not in samples as the
//! TRM's formula reads. Each 4094 bytes of samples take two 4080-byte descriptors, about twice their size
//! in buffer. The samples stay in order and each descriptor's length says how many it holds, so the
//! buffer is 2.5 times the capture plus two descriptors, and the capture ends once the descriptors
//! handed back hold enough.
//!
//! Setting RXEOF_NUM to 4080 would line the EOFs up with the descriptors and drop the overhead, but
//! esp-hal writes it inside `read()` after resetting the receiver, so changing it means a raw register
//! write racing the first EOF. That hasn't been tried on a box; the PSRAM is cheap (1.2 MB for 5 s).
//!
//! A ring that had to be relinked by this task before the DMA reached its end stalled for good on the box
//! (`STALLED`); one linear buffer doesn't depend on the main loop. Each capture also reports `MIC DEBUG`
//! lines 100 ms in and at the end: the descriptors and bytes so far, the first descriptor lengths, the I2S
//! and DMA registers, and how often the data pin changed level.

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
const GPIO_IN1: u32 = 0x6000_4040;

/// How often GPIO47 (DIN) changed level over 2000 quick reads. Only DIN: BCLK and WS are outputs whose
/// input path is off, so their input register reads a constant 0 however fast they run.
fn din_activity() -> u32 {
    let sample = || register(GPIO_IN1) >> 15 & 1;
    let mut last = sample();
    let mut changes = 0;
    for _ in 0..2000 {
        let now = sample();
        changes += (now != last) as u32;
        last = now;
    }
    changes
}

/// The descriptors the DMA has handed back, in chain order, and the sample bytes they hold.
fn received(first: *const DmaDescriptor, linked: usize) -> (usize, usize) {
    let mut bytes = 0;
    for index in 0..linked {
        // SAFETY: the descriptors live for the capture; the DMA may write them, so read them volatile.
        let flags = unsafe { core::ptr::read_volatile(first.add(index)).flags };
        if flags.owner() {
            return (index, bytes);
        }
        bytes += flags.length() as usize;
    }
    (linked, bytes)
}

/// One line of the receive path's state: descriptors handed back and the bytes they hold, the lengths of
/// the first six, then I2S0 RX_CONF, TX_CONF, INT_RAW, RX_CLKM_CONF, RXEOF_NUM, TX state, and GDMA channel
/// 1's IN_INT_RAW, INFIFO_STATUS, IN_LINK, IN_STATE, IN_DSCR, IN_PERI_SEL, OUT_STATE, OUT_PERI_SEL, then
/// the data pin's level changes.
fn note(stage: &str, first: *const DmaDescriptor, linked: usize) {
    let (done, bytes) = received(first, linked);
    let mut line = String::new();
    let _ = write!(line, "MIC DEBUG {stage} DESC {done}/{linked} BYTES {bytes} LENS");
    for index in 0..linked.min(6) {
        // SAFETY: as in `received`.
        let _ = write!(line, " {}", unsafe { core::ptr::read_volatile(first.add(index)).flags.length() });
    }
    let din = din_activity();
    let _ = write!(
        line,
        " I2S RXCONF {:08X} TXCONF {:08X} INTRAW {:08X} RXCLKM {:08X} RXEOF {:08X} TXSTATE {:08X} \
         DMA ININT {:08X} INFIFO {:08X} INLINK {:08X} INSTATE {:08X} INDSCR {:08X} INPERI {:08X} OUTSTATE {:08X} OUTPERI {:08X} \
         DIN {din}",
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
    // Room for the descriptors an EOF cuts short (see the top of this file).
    let size = (wanted * 5 / 2 + 2 * DESCRIPTOR_BYTES).next_multiple_of(ALIGN);
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
    // How many descriptors esp-hal linked.
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
        let (done, bytes) = received(first, linked);
        if bytes >= wanted || done == linked {
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
