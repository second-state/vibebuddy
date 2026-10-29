//! Two serial ports: UART0 (to the box's CH343 bridge) and the chip's own USB Serial/JTAG.
//! The Mac may plug into either, so both are read and both are written.
//!
//! UART has no flow control, and at 115200 baud the 128-byte hardware FIFO fills in 11 ms (1.4 ms at 921600);
//! drawing a frame or writing flash takes longer than that, so reception runs in an interrupt that moves
//! bytes into a 4 KB ring buffer for the main loop to take. Sending goes the other way through a ring of its own,
//! which the same interrupt feeds into the FIFO each time it runs empty, so the main loop never waits on the wire
//! unless the ring is full. USB has flow control and the host waits while the device is not reading, so reading it
//! in an async task is enough.
//!
//! The UART starts at 115200 and moves to a faster rate when the computer on the bridge asks
//! ([`vibebuddy_firmware_core::uart_rate`]).

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use critical_section::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use esp_hal::Blocking;
use esp_hal::handler;
use esp_hal::time::{Duration, Instant};
use esp_hal::uart::{Config, RxConfig, RxError, Uart, UartInterrupt};
use esp_hal::usb::usb_serial_jtag::{UsbSerialJtagRx, UsbSerialJtagTx};

/// A voice pack line is close to 1 KB; leave room for a few lines. The same for sending: at 921600 a `VOICE DATA`
/// line clears the wire in about 9 ms.
const UART_RING_BYTES: usize = 4096;

struct Ring {
    bytes: [u8; UART_RING_BYTES],
    head: usize,
    length: usize,
}

impl Ring {
    const fn new() -> Self {
        Self { bytes: [0; UART_RING_BYTES], head: 0, length: 0 }
    }

    fn room(&self) -> usize {
        UART_RING_BYTES - self.length
    }

    /// Drop what does not fit: the Mac treats the overflowed line as a bad line, and the next heartbeat
    /// recovers on its own. Returns how much was taken.
    fn push(&mut self, data: &[u8]) -> usize {
        let count = data.len().min(self.room());
        for &byte in &data[..count] {
            let tail = (self.head + self.length) % UART_RING_BYTES;
            self.bytes[tail] = byte;
            self.length += 1;
        }
        count
    }

    fn pop_into(&mut self, out: &mut [u8]) -> usize {
        let count = self.length.min(out.len());
        for slot in out.iter_mut().take(count) {
            *slot = self.bytes[self.head];
            self.head = (self.head + 1) % UART_RING_BYTES;
        }
        self.length -= count;
        count
    }
}

static UART: Mutex<RefCell<Option<Uart<'static, Blocking>>>> = Mutex::new(RefCell::new(None));
static UART_RING: Mutex<RefCell<Ring>> = Mutex::new(RefCell::new(Ring::new()));
static UART_TX_RING: Mutex<RefCell<Ring>> = Mutex::new(RefCell::new(Ring::new()));
/// The transmitter sent everything it was given and the send ring is empty: the rate may change.
static UART_TX_IDLE: AtomicBool = AtomicBool::new(true);
/// Bytes received that the UART couldn't frame since the main loop last looked: a sign the computer talks at
/// another rate.
static UART_RX_ERRORS: AtomicU32 = AtomicU32::new(0);

/// The UART's settings at `rate`. The RX FIFO hands over at half full: at 921600 that leaves 0.7 ms for the
/// interrupt to run, where the default (120 of 128 bytes) would leave 70 µs.
pub fn uart_config(rate: u32) -> Config {
    Config::default().with_baudrate(rate).with_rx(RxConfig::default().with_fifo_full_threshold(64))
}

/// Moves what the send ring holds into the FIFO, as much as fits.
fn feed_fifo(uart: &mut Uart<'static, Blocking>, ring: &mut Ring) {
    let mut chunk = [0u8; 128];
    while ring.length > 0 && uart.write_ready() {
        // Peek, write what the FIFO takes, then drop that much from the ring.
        let head = ring.head;
        let length = ring.length;
        let count = length.min(chunk.len());
        for (index, slot) in chunk.iter_mut().take(count).enumerate() {
            *slot = ring.bytes[(head + index) % UART_RING_BYTES];
        }
        let written = uart.write(&chunk[..count]).unwrap_or(count);
        ring.head = (head + written) % UART_RING_BYTES;
        ring.length -= written;
    }
}

#[handler]
fn uart_interrupt() {
    critical_section::with(|cs| {
        let mut uart = UART.borrow_ref_mut(cs);
        let Some(uart) = uart.as_mut() else {
            return;
        };
        let mut chunk = [0u8; 128];
        loop {
            match uart.read_buffered(&mut chunk) {
                Ok(0) => break,
                Err(RxError::FrameFormatViolated | RxError::GlitchOccurred | RxError::ParityMismatch) => {
                    UART_RX_ERRORS.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(_) => break,
                Ok(count) => {
                    UART_RING.borrow_ref_mut(cs).push(&chunk[..count]);
                }
            }
        }
        if uart.interrupts().contains(UartInterrupt::TxDone) {
            // Cleared before feeding, so the FIFO running dry again raises it again.
            uart.clear_interrupts(UartInterrupt::TxDone.into());
            let mut ring = UART_TX_RING.borrow_ref_mut(cs);
            if ring.length == 0 {
                UART_TX_IDLE.store(true, Ordering::Release);
            } else {
                feed_fifo(uart, &mut ring);
            }
        }
        uart.clear_interrupts(UartInterrupt::RxFifoFull | UartInterrupt::RxTimeout);
    });
}

/// Queues bytes to send, as many as the send ring has room for, and starts the transmitter if it is idle.
fn queue_uart(data: &[u8]) -> usize {
    critical_section::with(|cs| {
        let mut uart = UART.borrow_ref_mut(cs);
        let Some(uart) = uart.as_mut() else {
            return data.len();
        };
        let mut ring = UART_TX_RING.borrow_ref_mut(cs);
        let taken = ring.push(data);
        if taken > 0 {
            UART_TX_IDLE.store(false, Ordering::Release);
            // A transmitter-done left over from before would mark the line idle while these bytes still go out.
            uart.clear_interrupts(UartInterrupt::TxDone.into());
            feed_fifo(uart, &mut ring);
        }
        taken
    })
}

/// Room in the send ring right now.
fn uart_room() -> usize {
    critical_section::with(|cs| UART_TX_RING.borrow_ref(cs).room())
}

/// Hands the UART to the interrupt: when the FIFO is nearly full or the line goes quiet briefly, the
/// received bytes move into the ring buffer; when the transmitter has sent everything, the send ring feeds it.
pub fn start_uart(mut uart: Uart<'static, Blocking>) {
    uart.set_interrupt_handler(uart_interrupt);
    critical_section::with(|cs| {
        uart.listen(UartInterrupt::RxFifoFull | UartInterrupt::RxTimeout | UartInterrupt::TxDone);
        UART.borrow_ref_mut(cs).replace(uart);
    });
}

/// Receive errors since the last call.
pub fn take_uart_errors() -> u32 {
    UART_RX_ERRORS.swap(0, Ordering::Relaxed)
}

/// Moves the UART to `rate` once everything already queued has gone out at the old one: the line saying so must
/// reach the computer before the rate changes under it. Waits at most a second (4 KB at 115200 takes 0.36 s).
pub fn set_uart_rate(rate: u32) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !UART_TX_IDLE.load(Ordering::Acquire) && Instant::now() < deadline {
        core::hint::spin_loop();
    }
    critical_section::with(|cs| {
        if let Some(uart) = UART.borrow_ref_mut(cs).as_mut() {
            let _ = uart.flush();
            let _ = uart.apply_config(&uart_config(rate));
        }
        UART_TX_RING.borrow_ref_mut(cs).length = 0;
        UART_TX_IDLE.store(true, Ordering::Release);
    });
    UART_RX_ERRORS.store(0, Ordering::Relaxed);
}

/// Takes the bytes the UART has received.
pub fn take_uart(out: &mut [u8]) -> usize {
    critical_section::with(|cs| UART_RING.borrow_ref_mut(cs).pop_into(out))
}

/// Bytes received over USB land here for the main loop to take. When full the read task waits, and
/// the host waits with it.
pub static USB_PIPE: Pipe<CriticalSectionRawMutex, 2048> = Pipe::new();

#[embassy_executor::task]
pub async fn usb_rx_task(mut rx: UsbSerialJtagRx<'static, esp_hal::Async>) {
    use embedded_io_async::Read;
    let mut chunk = [0u8; 64];
    loop {
        if let Ok(count) = rx.read(&mut chunk).await
            && count > 0
        {
            USB_PIPE.write_all(&chunk[..count]).await;
        }
    }
}

/// Takes the bytes received over USB, without waiting.
pub fn take_usb(out: &mut [u8]) -> usize {
    USB_PIPE.try_read(out).unwrap_or(0)
}

/// Both outputs. Each is only a diagnostic channel, and neither may hold up the main loop.
pub struct Transport {
    usb: UsbSerialJtagTx<'static, esp_hal::Async>,
    /// No host is taking data on the USB side: the FIFO stays full. Later output is dropped until the
    /// FIFO accepts writes again, so every line does not wait in vain.
    usb_stalled: bool,
}

/// How long to wait at most when USB's 64-byte FIFO is full. A reading host always drains it within
/// 1 ms.
const USB_PATIENCE: Duration = Duration::from_millis(3);

impl Transport {
    pub fn new(usb: UsbSerialJtagTx<'static, esp_hal::Async>) -> Self {
        Self { usb, usb_stalled: false }
    }

    pub fn write(&mut self, data: &[u8]) {
        self.write_uart(data);
        self.write_usb(data);
    }

    /// UART sends bytes out at the baud rate whether or not anyone is listening, so waiting for room in the send
    /// ring is bounded: the interrupt empties it at the wire's pace.
    pub fn write_uart(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            data = &data[queue_uart(data)..];
        }
    }

    /// One line of an audio stream on the UART, whole or not at all: it waits for nothing.
    pub fn write_uart_stream(&mut self, line: &[u8]) {
        if uart_room() >= line.len() {
            self.write_uart(line);
        }
    }

    /// USB alone, for a stream the UART can't keep up with.
    pub fn write_usb(&mut self, data: &[u8]) {
        for &byte in data {
            if self.usb.write_byte_nb(byte).is_ok() {
                self.usb_stalled = false;
                continue;
            }
            if self.usb_stalled {
                return;
            }
            let _ = self.usb.flush_tx_nb();
            let deadline = Instant::now() + USB_PATIENCE;
            loop {
                if self.usb.write_byte_nb(byte).is_ok() {
                    break;
                }
                if Instant::now() > deadline {
                    self.usb_stalled = true;
                    return;
                }
            }
        }
        let _ = self.usb.flush_tx_nb();
    }
}
