//! Vibe Buddy 固件入口：把 ATK-DNESP32S3-BOX 的硬件接到 firmware-core 上。
//!
//! 板上的连线（与 C 固件一致）：
//! - LCD：ST7789，8 位并口。CS GPIO1、DC GPIO2、RD GPIO41、WR GPIO42，
//!   D0–D7 = GPIO40、39、38、12、11、10、9、46。
//! - I2C0：SDA GPIO48、SCL GPIO45。上面挂着 XL9555 扩展口（0x20）与 ES8311
//!   codec（0x18，NS4168 版本没有）。
//! - XL9555 P0：bit7 LCD 背光、bit5 功放使能、bit4 K1、bit3 K2（按键低有效）。
//! - K0：BOOT 键，GPIO0，低有效。
//! - I2S0：BCLK GPIO21、WS GPIO13、DOUT GPIO14，不用 MCLK。
//! - UART0：TX GPIO43、RX GPIO44，接 CH343 桥；另有芯片自带的 USB Serial/JTAG。
#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal 的类型常常持有正在传输的缓冲，forget 它们不安全")]

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

/// panic 之后重启，和 C 固件（ESP-IDF 默认 panic 即复位）一样。esp-backtrace
/// 先把回溯打出来，再调这里；不接管的话它关中断死循环，盒子冻结到断电。
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

/// 构建标识：git 描述 + 构建时刻，由 build.rs 写进来。
const BUILD: &str = env!("VIBEBUDDY_FW_BUILD");

/// 同一个标识以固定前缀、NUL 结尾留在镜像里：打包脚本从 .bin 里把它找出来
/// 写成 build.txt，App 拿它和盒子报的 `DISPLAY READY BUILD` 逐字比对。
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

    /// 读改写输出口的一位。
    fn set_output(&mut self, mask: u8, on: bool) -> Result<(), I2cError> {
        let output = self.read(XL9555_OUTPUT_PORT0)?;
        self.write(XL9555_OUTPUT_PORT0, if on { output | mask } else { output & !mask })
    }

    /// 读改写方向寄存器：置 1 为输入。
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
    /// RD 脚一直拉高：只写不读。放在这里是为了让它活着。
    _lcd_read: Output<'static>,
    /// ES8311 版本才有 codec；NS4168 版本没有音量可调。
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
        // 有 ES8311 就是 ES8311 版本；地址没人应答就是 NS4168 版本。
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
}

#[allow(clippy::large_stack_frames, reason = "main 里本来就要摆一堆外设和缓冲")]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // JSON 解析与一帧画面里的几行文字都在堆上，64 KB 足够。
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // 串口先起来：后面每一步的结果都要报给 Mac 端。
    let uart = Uart::new(peripherals.UART0, UartConfig::default().with_baudrate(115_200))
        .expect("UART0 配置")
        .with_tx(peripherals.GPIO43)
        .with_rx(peripherals.GPIO44);
    serial::start_uart(uart);
    let (usb_rx, usb_tx) = UsbSerialJtag::new(peripherals.USB_DEVICE).into_async().split();
    spawner.spawn(serial::usb_rx_task(usb_rx).expect("USB 读任务"));

    storage::install(FlashStorage::new(peripherals.FLASH));

    let i2c = I2c::new(peripherals.I2C0, I2cConfig::default().with_frequency(Rate::from_khz(400)))
        .expect("I2C0 配置")
        .with_sda(peripherals.GPIO48)
        .with_scl(peripherals.GPIO45);

    let lcd_read = Output::new(peripherals.GPIO41, Level::High, OutputConfig::default());
    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    // WR 空闲为高、下降沿送数据，与 ESP-IDF i80 的默认（pclk_idle_low = 0、
    // pclk_active_neg = 0）一致；esp-hal 的默认是空闲为低。
    let clock = ClockMode { polarity: Polarity::IdleHigh, phase: Phase::ShiftLow };
    let bus = I8080::new(lcd_cam.lcd, peripherals.DMA_CH0, I8080Config::default().with_frequency(Rate::from_mhz(10)).with_clock_mode(clock))
        .expect("LCD i8080 配置")
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
    let parameters = dma_tx_buffer!(16).expect("LCD 参数缓冲");
    let frame: DmaTxBuf = dma_tx_buffer!(FRAME_BYTES).expect("LCD 帧缓冲");
    let lcd = Lcd::new(bus, parameters, frame);

    // I2S 先开流（全是零），和 C 固件一样在配 codec 之前时钟就已经在走。
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH1,
        TdmConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("I2S0 配置")
    .into_async();
    let i2s_tx = i2s.i2s_tx.with_bclk(peripherals.GPIO21).with_ws(peripherals.GPIO13).with_dout(peripherals.GPIO14).build();
    let stream: DmaTxStreamBuf = dma_tx_stream_buffer!(STREAM_BYTES, STREAM_CHUNK);
    spawner.spawn(audio::audio_task(i2s_tx, stream).expect("音频任务"));

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

    // 让音频任务先跑一轮把 I2S 流开起来：ES8311 以 BCLK 为时钟，C 固件也是
    // 先使能 I2S 再配 codec。
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
        // 这里的等待就是按键的采样周期：太长会漏掉短促的轻点。
        Timer::after_millis(10).await;
    }
}
