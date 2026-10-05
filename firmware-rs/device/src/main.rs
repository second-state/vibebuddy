//! Vibe Buddy firmware entry point: wires the ATK-DNESP32S3-BOX hardware to firmware-core.
//!
//! Board wiring (same as the C firmware):
//! - LCD: ST7789, 8-bit parallel. CS GPIO1, DC GPIO2, RD GPIO41, WR GPIO42,
//!   D0-D7 = GPIO40, 39, 38, 12, 11, 10, 9, 46.
//! - I2C0: SDA GPIO48, SCL GPIO45. Carries the XL9555 expander (0x20) and the ES8311
//!   codec (0x18, absent on the NS4168 variant).
//! - XL9555 P0: bit7 LCD backlight, bit5 amplifier enable, bit4 K1, bit3 K2 (keys active low).
//! - K0: BOOT key, GPIO0, active low.
//! - I2S0: BCLK GPIO21, WS GPIO13, DOUT GPIO14, no MCLK.
//! - UART0: TX GPIO43, RX GPIO44, to the CH343 bridge; the chip's own USB Serial/JTAG as well.
#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal types often hold buffers mid-transfer; forgetting them is unsound")]

extern crate alloc;

mod audio;
mod lcd;
mod serial;
mod storage;

use core::sync::atomic::Ordering;

use embassy_executor::Spawner;
use embassy_time::{Instant, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::dma::{DmaTxBuf, DmaTxStreamBuf};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{AcknowledgeCheckFailedReason, Config as I2cConfig, Error as I2cError, I2c};
use esp_hal::i2s::master::{Channels, DataFormat, I2s, TdmConfig};
use esp_hal::lcd_cam::LcdCam;
use esp_hal::lcd_cam::lcd::{ClockMode, Phase, Polarity};
use esp_hal::lcd_cam::lcd::i8080::{Config as I8080Config, I8080};
use esp_hal::rng::Rng;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_hal::{Blocking, dma_tx_buffer, dma_tx_stream_buffer};
use esp_storage::FlashStorage;
use vibebuddy_firmware_core::audio::{self as codec, ES8311_ADDRESS, Prompt, Registers, SAMPLE_RATE};
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::Flash;
use vibebuddy_firmware_core::voices::ClipTable;

use crate::audio::{PLAYING, PlayCommand, QUEUE, STOP, STREAM_BYTES, STREAM_CHUNK};
use crate::lcd::Lcd;
use crate::serial::Transport;
use crate::storage::SharedFlash;

esp_bootloader_esp_idf::esp_app_desc!();

/// Reboot after a panic, like the C firmware (ESP-IDF resets on panic by default). esp-backtrace
/// prints the backtrace first and then calls this; left alone it disables interrupts and spins, and
/// the box freezes until power is cut.
#[unsafe(no_mangle)]
fn custom_halt() -> ! {
    esp_hal::system::software_reset()
}

const XL9555_ADDRESS: u8 = 0x20;
const XL9555_INPUT_PORT0: u8 = 0x00;
const XL9555_OUTPUT_PORT0: u8 = 0x02;
const XL9555_CONFIG_PORT0: u8 = 0x06;
const XL9555_LCD_BACKLIGHT: u8 = 0x80;
const XL9555_SPEAKER: u8 = 0x20;
const XL9555_K1: u8 = 0x10;
const XL9555_K2: u8 = 0x08;

/// Build stamp: git description plus build time, written in by build.rs.
const BUILD: &str = env!("VIBEBUDDY_FW_BUILD");

/// The same stamp stays in the image with a fixed prefix and a NUL terminator: the packaging script
/// finds it in the .bin and writes build.txt, which the App compares byte for byte with the box's
/// `DISPLAY READY BUILD`.
#[used]
static BUILD_MARKER: &[u8] = concat!("VIBEBUDDY-BUILD:", env!("VIBEBUDDY_FW_BUILD"), "\0").as_bytes();

struct Expander<'a> {
    i2c: &'a mut I2c<'static, Blocking>,
}

impl Expander<'_> {
    fn read(&mut self, register: u8) -> Result<u8, I2cError> {
        let mut value = [0u8];
        self.i2c.write_read(XL9555_ADDRESS, &[register], &mut value)?;
        Ok(value[0])
    }

    fn write(&mut self, register: u8, value: u8) -> Result<(), I2cError> {
        self.i2c.write(XL9555_ADDRESS, &[register, value])
    }

    /// Read-modify-write one bit of the output port.
    fn set_output(&mut self, mask: u8, on: bool) -> Result<(), I2cError> {
        let output = self.read(XL9555_OUTPUT_PORT0)?;
        self.write(XL9555_OUTPUT_PORT0, if on { output | mask } else { output & !mask })
    }

    /// Read-modify-write the direction register: 1 means input.
    fn set_direction(&mut self, mask: u8, input: bool) -> Result<(), I2cError> {
        let direction = self.read(XL9555_CONFIG_PORT0)?;
        self.write(XL9555_CONFIG_PORT0, if input { direction | mask } else { direction & !mask })
    }
}

struct Es8311<'a> {
    i2c: &'a mut I2c<'static, Blocking>,
}

impl Registers for Es8311<'_> {
    type Error = I2cError;

    fn read(&mut self, register: u8) -> Result<u8, I2cError> {
        let mut value = [0u8];
        self.i2c.write_read(ES8311_ADDRESS, &[register], &mut value)?;
        Ok(value[0])
    }

    fn write(&mut self, register: u8, value: u8) -> Result<(), I2cError> {
        self.i2c.write(ES8311_ADDRESS, &[register, value])
    }
}

struct DeviceBoard {
    i2c: I2c<'static, Blocking>,
    lcd: Lcd,
    transport: Transport,
    flash: SharedFlash,
    k0: Input<'static>,
    /// The RD pin stays high: write-only. Kept here so it stays alive.
    _lcd_read: Output<'static>,
    /// Only the ES8311 variant has a codec; the NS4168 variant has no adjustable volume.
    has_codec: bool,
}

impl DeviceBoard {
    fn expander(&mut self) -> Expander<'_> {
        Expander { i2c: &mut self.i2c }
    }

    fn codec(&mut self) -> Es8311<'_> {
        Es8311 { i2c: &mut self.i2c }
    }
}

impl Screen for DeviceBoard {
    fn frame(&mut self) -> &mut [u8] {
        self.lcd.frame()
    }

    fn present(&mut self) -> Result<(), ()> {
        self.lcd.present()
    }

    fn set_backlight(&mut self, on: bool) -> Result<(), ()> {
        self.expander().set_output(XL9555_LCD_BACKLIGHT, on).map_err(|_| ())
    }
}

impl Board for DeviceBoard {
    fn now_ms(&self) -> u32 {
        Instant::now().as_millis() as u32
    }

    fn write(&mut self, bytes: &[u8]) {
        self.transport.write(bytes);
    }

    fn flash(&mut self) -> &mut dyn Flash {
        &mut self.flash
    }

    fn with_frame_and_output(&mut self, action: &mut FrameAction) {
        let frame = self.lcd.frame();
        let transport = &mut self.transport;
        action(frame, &mut |bytes| transport.write(bytes));
    }

    fn init_display(&mut self) -> bool {
        let backlight_off = self
            .expander()
            .set_direction(XL9555_LCD_BACKLIGHT, false)
            .and_then(|()| self.expander().set_output(XL9555_LCD_BACKLIGHT, false));
        backlight_off.is_ok() && self.lcd.init().is_ok()
    }

    fn init_audio(&mut self, volume: u32) -> AudioStatus {
        // An ES8311 means the ES8311 variant; no answer at the address means the NS4168 variant.
        let mut chip_id = [0u8];
        match self.i2c.write_read(ES8311_ADDRESS, &[0xFD], &mut chip_id) {
            Ok(()) => {
                codec::start_es8311(&mut self.codec(), volume).map_err(|_| "ES8311 OPEN")?;
                self.has_codec = true;
            }
            Err(I2cError::AcknowledgeCheckFailed(AcknowledgeCheckFailedReason::Address | AcknowledgeCheckFailedReason::Unknown)) => {}
            Err(_) => return Err("ES8311 PROBE"),
        }
        let speaker = self
            .expander()
            .set_direction(XL9555_SPEAKER, false)
            .and_then(|()| self.expander().set_output(XL9555_SPEAKER, true));
        if speaker.is_err() {
            return Err("SPEAKER ENABLE");
        }
        Ok(if self.has_codec { "ES8311" } else { "NS4168" })
    }

    fn init_buttons(&mut self) -> Option<Levels> {
        self.expander().set_direction(XL9555_K1 | XL9555_K2, true).ok()?;
        let port = self.expander().read(XL9555_INPUT_PORT0).ok()?;
        Some(Levels { k0: self.k0.is_low(), k1: port & XL9555_K1 == 0, k2: port & XL9555_K2 == 0 })
    }

    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>) {
        let k0 = self.k0.is_low();
        let expander = self.expander().read(XL9555_INPUT_PORT0).ok().map(|port| (port & XL9555_K1 == 0, port & XL9555_K2 == 0));
        (k0, expander)
    }

    fn play(&mut self, prompt: Prompt, clips: ClipTable) -> Result<(), ()> {
        QUEUE.try_send(PlayCommand { prompt, clips }).map_err(|_| ())
    }

    fn stop_audio(&mut self) {
        QUEUE.clear();
        STOP.store(true, Ordering::Release);
    }

    fn audio_busy(&self) -> bool {
        !QUEUE.is_empty() || PLAYING.load(Ordering::Acquire) || STOP.load(Ordering::Acquire)
    }

    fn set_volume(&mut self, level: u32) -> Result<(), VolumeError> {
        if !self.has_codec {
            return Err(VolumeError::NotSupported);
        }
        codec::set_es8311_volume(&mut self.codec(), level).map_err(|_| VolumeError::Failed)
    }

    fn boot_other_app(&mut self) -> bool {
        let Some(running) = storage::running_app_offset() else {
            self.write(b"SWITCH APP NO RUNNING PARTITION\n");
            return false;
        };
        match vibebuddy_firmware_core::storage::boot_other_app(&mut SharedFlash, running) {
            Ok(true) => {}
            Ok(false) => {
                let line = vibebuddy_firmware_core::text!(48, "SWITCH APP NO OTHER APP RUNNING {:x}\n", running);
                self.write(line.as_bytes());
                return false;
            }
            Err(_) => {
                self.write(b"SWITCH APP FLASH ERROR\n");
                return false;
            }
        }
        // Let the SWITCH APP line out before the reset.
        let start = self.now_ms();
        while self.now_ms().wrapping_sub(start) < 50 {}
        esp_hal::system::software_reset()
    }

    fn has_other_app(&mut self) -> bool {
        let Some(running) = storage::running_app_offset() else { return false };
        matches!(vibebuddy_firmware_core::storage::other_app(&mut SharedFlash, running), Ok(Some(_)))
    }
}

#[allow(clippy::large_stack_frames, reason = "main has to hold a lot of peripherals and buffers anyway")]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // JSON parsing and the few lines of text in a frame live on the heap; 72 KB is enough.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // Serial comes up first: the result of every later step is reported to the Mac.
    let uart = Uart::new(peripherals.UART0, UartConfig::default().with_baudrate(115_200))
        .expect("UART0 config")
        .with_tx(peripherals.GPIO43)
        .with_rx(peripherals.GPIO44);
    serial::start_uart(uart);
    let (usb_rx, usb_tx) = UsbSerialJtag::new(peripherals.USB_DEVICE).into_async().split();
    spawner.spawn(serial::usb_rx_task(usb_rx).expect("USB read task"));

    storage::install(FlashStorage::new(peripherals.FLASH));

    let i2c = I2c::new(peripherals.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(400)))
        .expect("I2C0 config")
        .with_sda(peripherals.GPIO48)
        .with_scl(peripherals.GPIO45);

    let lcd_read = Output::new(peripherals.GPIO41, Level::High, OutputConfig::default());
    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    // WR idles high and data goes out on the falling edge, matching the ESP-IDF i80 defaults
    // (pclk_idle_low = 0, pclk_active_neg = 0); esp-hal defaults to idle low.
    let clock = ClockMode { polarity: Polarity::IdleHigh, phase: Phase::ShiftLow };
    let bus = I8080::new(lcd_cam.lcd, peripherals.DMA_CH0, I8080Config::default().with_frequency(Rate::from_mhz(10)).with_clock_mode(clock))
        .expect("LCD i8080 config")
        .with_cs(peripherals.GPIO1)
        .with_dc(peripherals.GPIO2)
        .with_wrx(peripherals.GPIO42)
        .with_data0(peripherals.GPIO40)
        .with_data1(peripherals.GPIO39)
        .with_data2(peripherals.GPIO38)
        .with_data3(peripherals.GPIO12)
        .with_data4(peripherals.GPIO11)
        .with_data5(peripherals.GPIO10)
        .with_data6(peripherals.GPIO9)
        .with_data7(peripherals.GPIO46);
    let parameters = dma_tx_buffer!(16).expect("LCD parameter buffer");
    let frame: DmaTxBuf = dma_tx_buffer!(FRAME_BYTES).expect("LCD frame buffer");
    let lcd = Lcd::new(bus, parameters, frame);

    // Start the I2S stream first (all zeros): as in the C firmware, the clock runs before the codec is
    // configured.
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH1,
        TdmConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("I2S0 config")
    .into_async();
    let i2s_tx = i2s.i2s_tx.with_bclk(peripherals.GPIO21).with_ws(peripherals.GPIO13).with_dout(peripherals.GPIO14).build();
    let stream: DmaTxStreamBuf = dma_tx_stream_buffer!(STREAM_BYTES, STREAM_CHUNK);
    spawner.spawn(audio::audio_task(i2s_tx, stream).expect("audio task"));

    let k0 = Input::new(peripherals.GPIO0, InputConfig::default().with_pull(Pull::Up));

    let mut board = DeviceBoard {
        i2c,
        lcd,
        transport: Transport::new(usb_tx),
        flash: SharedFlash,
        k0,
        _lcd_read: lcd_read,
        has_codec: false,
    };

    // Let the audio task run once to start the I2S stream: the ES8311 is clocked by BCLK, and the C
    // firmware also enables I2S before configuring the codec.
    Timer::after_millis(20).await;

    let seed = Rng::new().random();
    let mut firmware = Firmware::new(seed, board.now_ms(), BUILD.as_bytes());
    firmware.boot(&mut board);

    let mut input = [0u8; 256];
    loop {
        loop {
            let count = serial::take_uart(&mut input);
            if count == 0 {
                break;
            }
            firmware.receive(&mut board, &input[..count]);
        }
        loop {
            let count = serial::take_usb(&mut input);
            if count == 0 {
                break;
            }
            firmware.receive(&mut board, &input[..count]);
        }
        firmware.poll(&mut board);
        // This wait is the key sampling period: too long would miss quick taps.
        Timer::after_millis(10).await;
    }
}
