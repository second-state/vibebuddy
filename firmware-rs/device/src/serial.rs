//! 两条串口：UART0（接盒子上的 CH343 桥）与芯片自带的 USB Serial/JTAG。
//! Mac 端插哪个口都行，所以两路都收、两路都写。
//!
//! UART 没有流控，115200 波特下 128 字节的硬件 FIFO 11 ms 就满；画一帧、
//! 写一次 flash 都比这久，所以接收放在中断里，搬进 4 KB 的环形缓冲，主循环
//! 再取走。USB 有流控，主机在设备不读时会等，放在一个异步任务里读就够了。

use core::cell::RefCell;

use critical_section::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pipe::Pipe;
use esp_hal::Blocking;
use esp_hal::handler;
use esp_hal::time::{Duration, Instant};
use esp_hal::uart::{Uart, UartInterrupt};
use esp_hal::usb::usb_serial_jtag::{UsbSerialJtagRx, UsbSerialJtagTx};

/// 语音包的一行接近 1 KB，给它留出几行的余量。
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

    /// 放不下的丢掉：溢出的那一行会被 Mac 端当成坏行，下一条心跳自然恢复。
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

/// 把 UART 交给中断：FIFO 快满或者线上停顿一下，就把收到的搬进环形缓冲。
pub fn start_uart(mut uart: Uart<'static, Blocking>) {
    uart.set_interrupt_handler(uart_interrupt);
    critical_section::with(|cs| {
        uart.listen(UartInterrupt::RxFifoFull | UartInterrupt::RxTimeout);
        UART.borrow_ref_mut(cs).replace(uart);
    });
}

/// 取走 UART 收到的字节。
pub fn take_uart(out: &mut [u8]) -> usize {
    critical_section::with(|cs| UART_RING.borrow_ref_mut(cs).pop_into(out))
}

/// USB 收到的字节先进这里，主循环取走。满了读任务就等，主机随之等。
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

/// 取走 USB 收到的字节，不等。
pub fn take_usb(out: &mut [u8]) -> usize {
    USB_PIPE.try_read(out).unwrap_or(0)
}

/// 两路输出。都只是诊断通道，谁都不许拖住主循环。
pub struct Transport {
    usb: UsbSerialJtagTx<'static, esp_hal::Async>,
    /// USB 那头没有主机取数据：FIFO 一直满。之后的输出先直接丢，等 FIFO
    /// 又能写了再恢复，免得每一行都白等一遍。
    usb_stalled: bool,
}

/// USB 的 64 字节 FIFO 满了最多等这么久。主机在读时 1 ms 内必然取走。
const USB_PATIENCE: Duration = Duration::from_millis(3);

impl Transport {
    pub fn new(usb: UsbSerialJtagTx<'static, esp_hal::Async>) -> Self {
        Self { usb, usb_stalled: false }
    }

    pub fn write(&mut self, data: &[u8]) {
        self.write_uart(data);
        self.write_usb(data);
    }

    /// UART 不管有没有人接，都按波特率把字节送出去，所以阻塞写是有界的。
    /// 每次只在临界区里塞一小段，中断仍能及时收走 RX FIFO。
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
