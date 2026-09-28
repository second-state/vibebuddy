//! Two serial ports: UART0 (to the box's CH343 bridge) and the chip's own USB Serial/JTAG.
//! The Mac may plug into either, so both are read and both are written.
//!
//! UART has no flow control, and at 115200 baud the 128-byte hardware FIFO fills in 11 ms; drawing
//! a frame or writing flash takes longer than that, so reception runs in an interrupt that moves
//! bytes into a 4 KB ring buffer for the main loop to take. USB has flow control and the host waits
//! while the device is not reading, so reading it in an async task is enough.

use core::cell::RefCell;

use critical_section::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use esp_hal::Blocking;
use esp_hal::handler;
use esp_hal::time::{Duration, Instant};
use esp_hal::uart::{Uart, UartInterrupt};
use esp_hal::usb::usb_serial_jtag::{UsbSerialJtagRx, UsbSerialJtagTx};

/// A voice pack line is close to 1 KB; leave room for a few lines.
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

    /// Drop what does not fit: the Mac treats the overflowed line as a bad line, and the next heartbeat
    /// recovers on its own.
    fn push(&mut self, data: &[u8]) {
        for &byte in data {
            if self.length == UART_RING_BYTES {
                return;
            }
            let tail = (self.head + self.length) % UART_RING_BYTES;
            self.bytes[tail] = byte;
            self.length += 1;
        }
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
                Ok(0) | Err(_) => break,
                Ok(count) => UART_RING.borrow_ref_mut(cs).push(&chunk[..count]),
            }
        }
        uart.clear_interrupts(UartInterrupt::RxFifoFull | UartInterrupt::RxTimeout);
    });
}

/// Hands the UART to the interrupt: when the FIFO is nearly full or the line goes quiet briefly, the
/// received bytes move into the ring buffer.
pub fn start_uart(mut uart: Uart<'static, Blocking>) {
    uart.set_interrupt_handler(uart_interrupt);
    critical_section::with(|cs| {
        uart.listen(UartInterrupt::RxFifoFull | UartInterrupt::RxTimeout);
        UART.borrow_ref_mut(cs).replace(uart);
    });
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

    /// UART sends bytes out at the baud rate whether or not anyone is listening, so a blocking write is
    /// bounded. Each critical section feeds only a small piece, so the interrupt can still drain the
    /// RX FIFO in time.
    fn write_uart(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let written = critical_section::with(|cs| {
                let mut uart = UART.borrow_ref_mut(cs);
                match uart.as_mut() {
                    Some(uart) => uart.write(&data[..data.len().min(16)]).unwrap_or(data.len()),
                    None => data.len(),
                }
            });
            data = &data[written..];
        }
    }

    fn write_usb(&mut self, data: &[u8]) {
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
