//! VibeBuddy firmware entry point: wires the ATK-DNESP32S3-BOX hardware to firmware-core.
//!
//! Board wiring (same as the C firmware):
//! - LCD: ST7789, 8-bit parallel. CS GPIO1, DC GPIO2, RD GPIO41, WR GPIO42,
//!   D0-D7 = GPIO40, 39, 38, 12, 11, 10, 9, 46.
//! - I2C0: SDA GPIO48, SCL GPIO45. Carries the XL9555 expander (0x20) and the ES8311
//!   codec (0x18, absent on the NS4168 variant).
//! - XL9555 P0: bit7 LCD backlight, bit5 amplifier enable, bit4 K1, bit3 K2 (keys active low).
//! - K0: BOOT key, GPIO0, active low.
//! - I2S0: BCLK GPIO21, WS GPIO13, DOUT GPIO14, no MCLK. DIN GPIO47 (the ES8311's ADC, left slot;
//!   verified on one box, docs/hardware.md; only read during the `device.mic.record` diagnostic).
//! - UART0: TX GPIO43, RX GPIO44, to the CH343 bridge; the chip's own USB Serial/JTAG as well.
#![no_std]
#![no_main]
#![deny(clippy::mem_forget, reason = "esp_hal types often hold buffers mid-transfer; forgetting them is unsound")]

extern crate alloc;

mod audio;
mod lcd;
mod mic;
mod net;
mod relay;
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
use esp_hal::rng::{Rng, Trng, TrngSource};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::usb::usb_serial_jtag::UsbSerialJtag;
use esp_hal::{Blocking, dma_tx_buffer, dma_tx_stream_buffer};
use esp_storage::FlashStorage;
use vibebuddy_firmware_core::audio::{self as codec, ES8311_ADDRESS, Registers, SAMPLE_RATE, Sound};
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::noise::{self, SealedLines, Sealer};
use vibebuddy_firmware_core::relay_box::{self, RelayBox};
use vibebuddy_firmware_core::storage::Flash;

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
    /// The computer linked over the network, which gets the box's output too, sealed for its session.
    lan_peer: Option<(u32, SealedLines)>,
    /// The session with a computer linked through the Relay: the box's output goes there as whole sealed lines.
    relay_peer: Option<SealedLines>,
    /// The firmware asked to restart; the main loop closes the sockets first.
    restart: bool,
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
        // A line with no room is dropped whole, before it is sealed (noise::SealedLines).
        if let Some((conn, lines)) = &mut self.lan_peer {
            let conn = *conn;
            lines.lines(
                bytes,
                |length| net::room(conn) > length,
                |line| {
                    net::send(conn, line.as_bytes());
                    net::send(conn, b"\n");
                },
            );
        }
        if let Some(lines) = &mut self.relay_peer {
            lines.lines(
                bytes,
                |_| relay::OUT.free_capacity() > 0,
                |line| {
                    let _ = relay::OUT.try_send((relay::current_socket(), relay_box::data(line)));
                },
            );
        }
    }

    fn lan_send(&mut self, conn: u32, bytes: &[u8]) {
        net::send(conn, bytes);
    }

    fn lan_close(&mut self, conn: u32) {
        net::close(conn);
    }

    fn lan_peer(&mut self, peer: Option<(u32, Sealer)>) {
        self.lan_peer = peer.map(|(conn, sealer)| (conn, SealedLines::new(sealer)));
    }

    fn lan_peer_send(&mut self, line: &[u8]) {
        if let Some((conn, lines)) = &mut self.lan_peer
            && net::room(*conn) > noise::sealed_len(line.len())
            && let Some(line) = lines.seal(line)
        {
            net::send(*conn, line.as_bytes());
            net::send(*conn, b"\n");
        }
    }

    fn relay_peer(&mut self, sealer: Option<Sealer>) {
        self.relay_peer = sealer.map(SealedLines::new);
    }

    fn peer_can_take(&self, bytes: usize) -> bool {
        // A Relay slot holds a line of any length; leave a couple for the box's other messages.
        self.lan_peer.as_ref().is_none_or(|(conn, _)| net::room(*conn) > noise::sealed_len(bytes)) && (self.relay_peer.is_none() || relay::OUT.free_capacity() > 2)
    }

    // With the radio on, the RNG draws on real noise.
    fn random(&mut self) -> Option<[u8; 32]> {
        let mut bytes = [0u8; 32];
        esp_hal::rng::Rng::new().read(&mut bytes);
        Some(bytes)
    }

    fn restart(&mut self) {
        self.restart = true;
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

    fn play(&mut self, sound: Sound) -> Result<(), ()> {
        QUEUE.try_send(PlayCommand { sound }).map_err(|_| ())
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

    fn i2c_read(&mut self, address: u8, register: u8) -> Option<u8> {
        let mut value = [0u8];
        self.i2c.write_read(address, &[register], &mut value).ok().map(|()| value[0])
    }

    fn i2c_write(&mut self, address: u8, register: u8, value: u8) -> Result<(), ()> {
        self.i2c.write(address, &[register, value]).map_err(|_| ())
    }

    fn mic_start(&mut self, bytes: usize) -> Result<(), ()> {
        mic::DONE.reset();
        mic::START.signal(bytes);
        Ok(())
    }

    fn mic_take(&mut self) -> Option<Result<alloc::vec::Vec<u8>, &'static str>> {
        while let Ok(note) = mic::NOTES.try_receive() {
            self.write(note.as_bytes());
            self.write(b"\n");
        }
        mic::DONE.try_take()
    }
}

#[allow(clippy::large_stack_frames, reason = "main has to hold a lot of peripherals and buffers anyway")]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // The module's 8 MB octal PSRAM comes first, so the firmware's own allocations (JSON, looks, network
    // buffers) land there and internal RAM stays for the Wi-Fi driver, which asks for internal memory
    // explicitly. The flash runs at 40 MHz (the bootloader's header), and the PSRAM timing must agree.
    esp_alloc::psram_allocator!(
        peripherals.PSRAM,
        esp_hal::psram,
        esp_hal::psram::PsramConfig { flash_frequency: esp_hal::psram::FlashFreq::FlashFreq40m, ..Default::default() }
    );
    // JSON parsing and the few lines of text in a frame live on the heap; 72 KB is enough.
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    // A Character's look lives on the heap too (about 6 KB, plus as much again while it is read from
    // flash); this region keeps it from eating into what JSON parsing needs.
    esp_alloc::heap_allocator!(size: 16 * 1024);

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
    // Wi-Fi starts idle; it joins a network once the firmware below names one.
    let stack = net::start(&spawner, peripherals.WIFI);

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
    // configured. The receiver shares the transmitter's BCLK and WS (signal loopback), as ESP-IDF sets up a
    // full-duplex channel pair on the same pins; it only runs during the microphone diagnostic.
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH1,
        TdmConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO)
            .with_signal_loopback(true),
    )
    .expect("I2S0 config")
    .into_async();
    let i2s_tx = i2s.i2s_tx.with_bclk(peripherals.GPIO21).with_ws(peripherals.GPIO13).with_dout(peripherals.GPIO14).build();
    let stream: DmaTxStreamBuf = dma_tx_stream_buffer!(STREAM_BYTES, STREAM_CHUNK);
    spawner.spawn(audio::audio_task(i2s_tx, stream).expect("audio task"));
    let i2s_rx = i2s.i2s_rx.with_din(peripherals.GPIO47).build();
    spawner.spawn(mic::mic_task(i2s_rx).expect("mic task"));

    let k0 = Input::new(peripherals.GPIO0, InputConfig::default().with_pull(Pull::Up));

    let mut board = DeviceBoard {
        i2c,
        lcd,
        transport: Transport::new(usb_tx),
        flash: SharedFlash,
        k0,
        _lcd_read: lcd_read,
        has_codec: false,
        lan_peer: None,
        relay_peer: None,
        restart: false,
    };

    // Let the audio task run once to start the I2S stream: the ES8311 is clocked by BCLK, and the C
    // firmware also enables I2S before configuring the codec.
    Timer::after_millis(20).await;

    let seed = Rng::new().random();
    // The box's pairing key is made from this on its first boot, so it must be true randomness: the
    // RNG only gets that with the radio on or the ADC sampling noise, and nothing else uses ADC1.
    // The source stays on: TLS for the Relay draws on the TRNG too.
    static TRNG_SOURCE: static_cell::StaticCell<TrngSource<'static>> = static_cell::StaticCell::new();
    TRNG_SOURCE.init(TrngSource::new(peripherals.RNG, peripherals.ADC1));
    static TRNG: static_cell::StaticCell<Trng> = static_cell::StaticCell::new();
    let trng = TRNG.init(Trng::try_new().expect("TRNG with the ADC entropy source"));
    let mut entropy = [0u8; 32];
    trng.read(&mut entropy);
    let mut firmware = Firmware::new(seed, board.now_ms(), BUILD.as_bytes(), env!("CARGO_PKG_VERSION")).with_entropy(entropy);
    firmware.boot(&mut board);
    // Booting got this far, so an image installed over the air keeps its slot.
    storage::confirm_running_image();
    net::CREDENTIALS.signal(wifi_credentials(&firmware));
    report_memory(&mut board);
    if let Some(stack) = stack {
        if let Some(link) = firmware.link() {
            net::advertise(&spawner, stack, vibebuddy_firmware_core::link::encode(&link.public_key()).into());
        }
        start_relay(&spawner, stack, trng, &firmware);
    }
    let mut relay_box = RelayBox::new();

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
        while let Ok(event) = net::EVENTS.try_receive() {
            match event {
                net::NetEvent::Line(line) => {
                    board.write(line.as_bytes());
                    board.write(b"\n");
                    if line == "WIFI CONNECTED" {
                        firmware.set_wifi_joined(true);
                    } else if line == "WIFI DISCONNECTED" || line.starts_with("WIFI FAILED") || line.starts_with("WIFI CONNECTING") {
                        firmware.set_wifi_joined(false);
                    }
                    // Once on the network, the Wi-Fi driver has taken what it needs (#108).
                    if let Some(address) = line.strip_prefix("WIFI ADDRESS ") {
                        firmware.set_wifi_address(address);
                        report_memory(&mut board);
                    }
                }
                net::NetEvent::Opened(conn, nonce) => firmware.lan_opened(&mut board, conn, nonce),
                net::NetEvent::Received(conn, bytes) => firmware.lan_received(&mut board, conn, &bytes),
                net::NetEvent::Closed(conn) => firmware.lan_closed(&mut board, conn),
            }
        }
        while let Ok(event) = relay::IN.try_receive() {
            match event {
                relay::RelayIn::Challenge(socket, nonce) => {
                    if let Some(hello) = relay_box.hello(&firmware, &nonce) {
                        let _ = relay::OUT.try_send((socket, hello));
                    }
                }
                relay::RelayIn::Message(text) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                        for message in relay_box.message(&mut firmware, &mut board, &value) {
                            let _ = relay::OUT.try_send((relay::current_socket(), message));
                        }
                    }
                }
                relay::RelayIn::Gone => relay_box.gone(&mut firmware, &mut board),
                relay::RelayIn::Line(line) => {
                    board.write(line.as_bytes());
                    board.write(b"\n");
                    // With TLS up, what is left (#108).
                    if line == "RELAY CONNECTED" {
                        report_memory(&mut board);
                    }
                }
            }
        }
        for message in relay_box.report(&firmware, board.now_ms()) {
            let _ = relay::OUT.try_send((relay::current_socket(), message));
        }
        if firmware.take_scan_request() {
            net::SCAN.signal(());
        }
        if firmware.take_relay_change() {
            relay::URL.signal(firmware.relay_url().into());
        }
        if firmware.take_wifi_change() {
            net::CREDENTIALS.signal(wifi_credentials(&firmware));
        }
        firmware.poll(&mut board);
        // Before restarting, close the Relay socket and the network connections so the computers and the
        // Relay see the box go at once, rather than after their silence timeouts; a new identity has a new
        // box-id, so nothing would replace the old one's socket (#122). The wait also lets the last line out.
        if board.restart {
            relay::STOP.signal(());
            net::close_all();
            Timer::after_millis(500).await;
            esp_hal::system::software_reset();
        }
        // This wait is the key sampling period: too long would miss quick taps.
        Timer::after_millis(10).await;
    }
}

fn wifi_credentials(firmware: &Firmware) -> Option<(alloc::string::String, alloc::string::String)> {
    firmware.wifi().map(|wifi| (wifi.ssid.clone(), wifi.password.clone()))
}

/// `MEMORY INTERNAL <free>/<total> PSRAM <free>/<total>` in bytes: what the heaps have left (#108).
fn report_memory(board: &mut DeviceBoard) {
    let stats = esp_alloc::HEAP.stats();
    let internal = esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::Internal.into());
    let external = esp_alloc::HEAP.free_caps(esp_alloc::MemoryCapability::External.into());
    let line = alloc::format!("MEMORY INTERNAL {internal} PSRAM {external} TOTAL {}/{}\n", stats.size - stats.current_usage, stats.size);
    board.write(line.as_bytes());
}

/// TLS for the Relay (ADR-0012): mbedtls with the TRNG, embassy's clock for timeouts, and the SNTP wall
/// clock for certificate dates. The task waits for Wi-Fi and the box's id.
fn start_relay(spawner: &Spawner, stack: embassy_net::Stack<'static>, trng: &'static mut Trng, firmware: &Firmware) {
    static TIMER: mbedtls_rs::sys::hook::backend::embassy::timer::EmbassyTimer = mbedtls_rs::sys::hook::backend::embassy::timer::EmbassyTimer;
    static CLOCK: relay::WallClock = relay::WallClock;
    // SAFETY: both hooks are statics, installed before any TLS session starts.
    unsafe {
        mbedtls_rs::sys::hook::timer::hook_timer(Some(&TIMER));
        mbedtls_rs::sys::hook::wall_clock::hook_wall_clock(Some(&CLOCK));
    }
    let Ok(tls) = mbedtls_rs::Tls::new(trng) else { return };
    let Some(link) = firmware.link() else { return };
    relay::BOX_ID.signal(vibebuddy_firmware_core::link::encode(&link.public_key()));
    relay::URL.signal(firmware.relay_url().into());
    spawner.spawn(relay::task(stack, tls).expect("Relay task"));
}
