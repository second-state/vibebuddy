//! ST7789, 8-bit parallel (i8080), sent through the LCD_CAM peripheral with DMA. The command
//! sequence lives in core's `lcd` module; this only sends it out.

use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::dma::{DmaTxBuf, DmaTxBuffer};
use esp_hal::lcd_cam::lcd::i8080::{I8080, I8080Transfer};
use esp_hal::time::{Duration, Instant};
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::lcd::{INIT_SEQUENCE, RAMWR, WINDOW};

/// How long to wait at most for one transfer. A full 150 KB screen takes about 15 ms at 10 MHz; the C
/// firmware waits 1 second. On timeout, cancel: a dark screen beats a hung main loop, and the Mac
/// still sees DISPLAY ERROR.
const TRANSFER_TIMEOUT: Duration = Duration::from_millis(1000);

type Bus = I8080<'static, Blocking>;

/// Waits for the transfer to finish, cancelling on timeout. Hands back the bus and buffer either way.
fn finish<B: DmaTxBuffer>(transfer: I8080Transfer<'static, B, Blocking>) -> (Result<(), ()>, Bus, B::Final) {
    let start = Instant::now();
    while !transfer.is_done() {
        if start.elapsed() > TRANSFER_TIMEOUT {
            let (bus, buffer) = transfer.cancel();
            return (Err(()), bus, buffer);
        }
    }
    let (result, bus, buffer) = transfer.wait();
    (result.map_err(|_| ()), bus, buffer)
}

pub struct Lcd {
    bus: Option<Bus>,
    /// Small buffer for commands with parameters.
    parameters: Option<DmaTxBuf>,
    frame: Option<DmaTxBuf>,
}

impl Lcd {
    pub fn new(bus: Bus, parameters: DmaTxBuf, frame: DmaTxBuf) -> Self {
        Self { bus: Some(bus), parameters: Some(parameters), frame: Some(frame) }
    }

    fn send(&mut self, command: u8, parameters: &[u8]) -> Result<(), ()> {
        // Commands without parameters (SWRESET, SLPOUT, INVON, DISPON) are sent with a dummy byte
        // too. esp-hal's i8080 always opens a data phase, and the data ends on DMA's EOF; an empty
        // buffer has no EOF and may never finish. ESP-IDF disables the data phase here and attaches
        // a fake buffer with EOF, and esp-hal cannot do the first half. The ST7789 ignores extra
        // parameter bytes after a command that takes none.
        let parameters = if parameters.is_empty() { &[0x00][..] } else { parameters };
        let bus = self.bus.take().ok_or(())?;
        let mut buffer = self.parameters.take().ok_or(())?;
        buffer.fill(parameters);
        let (result, bus, buffer) = match bus.send(command, 0, buffer) {
            Ok(transfer) => finish(transfer),
            Err((_, bus, buffer)) => (Err(()), bus, buffer),
        };
        self.bus = Some(bus);
        self.parameters = Some(buffer);
        result
    }

    /// Initializes in the order the C firmware sends through esp_lcd and turns the display on; the
    /// backlight is not handled here.
    pub fn init(&mut self) -> Result<(), ()> {
        let delay = Delay::new();
        for step in INIT_SEQUENCE {
            self.send(step.command, step.parameters)?;
            if step.delay_ms > 0 {
                delay.delay_millis(step.delay_ms);
            }
        }
        Ok(())
    }

    pub fn frame(&mut self) -> &mut [u8] {
        match self.frame.as_mut() {
            Some(frame) => &mut frame.as_mut_slice()[..FRAME_BYTES],
            // send and finish hand the buffer back whether they succeed or not, so this is unreachable.
            None => panic!("LCD frame buffer lost"),
        }
    }

    /// Full-screen refresh: set the window, then RAMWR with the whole frame buffer. Returns once DMA is
    /// done.
    pub fn present(&mut self) -> Result<(), ()> {
        for step in &WINDOW {
            self.send(step.command, step.parameters)?;
        }
        let bus = self.bus.take().ok_or(())?;
        let mut frame = self.frame.take().ok_or(())?;
        frame.set_length(FRAME_BYTES);
        let (result, bus, frame) = match bus.send(RAMWR, 0, frame) {
            Ok(transfer) => finish(transfer),
            Err((_, bus, frame)) => (Err(()), bus, frame),
        };
        self.bus = Some(bus);
        self.frame = Some(frame);
        result
    }
}
