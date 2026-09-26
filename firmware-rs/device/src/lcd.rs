//! ST7789，8 位并口（i8080），经 LCD_CAM 外设加 DMA 发送。命令序列在
//! core 的 `lcd` 模块里，这里只负责把它发出去。

use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::dma::{DmaTxBuf, DmaTxBuffer};
use esp_hal::lcd_cam::lcd::i8080::{I8080, I8080Transfer};
use esp_hal::time::{Duration, Instant};
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::lcd::{INIT_SEQUENCE, RAMWR, WINDOW};

/// 一次传输最多等这么久。整屏 150 KB 在 10 MHz 下约 15 ms；C 固件等 1 秒。
/// 等不到就取消：屏幕黑着也比主循环卡死强，Mac 端还能看到 DISPLAY ERROR。
const TRANSFER_TIMEOUT: Duration = Duration::from_millis(1000);

type Bus = I8080<'static, Blocking>;

/// 等传输结束，超时就取消。无论成败都把总线与缓冲交回来。
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
    /// 带参数命令用的小缓冲。
    parameters: Option<DmaTxBuf>,
    frame: Option<DmaTxBuf>,
}

impl Lcd {
    pub fn new(bus: Bus, parameters: DmaTxBuf, frame: DmaTxBuf) -> Self {
        Self { bus: Some(bus), parameters: Some(parameters), frame: Some(frame) }
    }

    fn send(&mut self, command: u8, parameters: &[u8]) -> Result<(), ()> {
        // 没有参数的命令（SWRESET、SLPOUT、INVON、DISPON）也带一个哑字节发。
        // esp-hal 的 i8080 每次都开数据阶段，数据结束要靠 DMA 的 EOF；空缓冲
        // 没有 EOF，可能永远等不到结束。ESP-IDF 在这里是关掉数据阶段、另挂一个
        // 带 EOF 的假缓冲，esp-hal 做不到前一半。ST7789 忽略无参数命令后面多出
        // 来的参数字节。
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

    /// 按 C 固件经 esp_lcd 发出的顺序初始化，开显示，但背光不归这里管。
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
            // send 与 finish 无论成败都会把缓冲交回来，走不到这里。
            None => panic!("LCD 帧缓冲丢了"),
        }
    }

    /// 整屏刷新：设窗口，RAMWR 加整块帧缓冲。等 DMA 发完再返回。
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
