//! 语音播放任务：I2S 以 DMA 流式缓冲连续输出，有句子就放句子，没有就放静音。
//!
//! C 固件的 I2S 开着 auto_clear，时钟一直在走、没数据就发零；ES8311 以 BCLK
//! 为时钟源，时钟断了再续上容易出爆音。这里一样：流一直开着，空闲时推零。
//! 主循环卡住（写 flash、刷屏）时流里还有约 170 ms 的余量；卡得更久（擦语音
//! 分区要好几秒）流会断，之后清零缓冲重新开始。

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use esp_hal::Async;
use esp_hal::dma::DmaTxStreamBuf;
use esp_hal::i2s::master::I2sTx;
use vibebuddy_firmware_core::audio::Prompt;
use vibebuddy_firmware_core::voices::{ClipTable, clip_index};

use crate::storage;

/// 流式缓冲：24 kHz × 双声道 × 16 bit = 96 KB/s，16 KB 约 170 ms。
pub const STREAM_BYTES: usize = 16000;
pub const STREAM_CHUNK: usize = 1000;

pub struct PlayCommand {
    pub prompt: Prompt,
    pub clips: ClipTable,
}

/// 和 C 固件一样，最多排 8 句。
pub static QUEUE: Channel<CriticalSectionRawMutex, PlayCommand, 8> = Channel::new();
/// 正在放一句（从队列取出到最后一个字节推进缓冲）。
pub static PLAYING: AtomicBool = AtomicBool::new(false);
/// 主循环要求立刻停：清掉正在放的，队列由请求方清。
pub static STOP: AtomicBool = AtomicBool::new(false);

static INPUT_REQUIRED: &[u8] = include_bytes!("../../../firmware/main/assets/input_required.pcm");
static DONE: &[u8] = include_bytes!("../../../firmware/main/assets/done.pcm");
static FAILED: &[u8] = include_bytes!("../../../firmware/main/assets/failed.pcm");
static FOCUS_DONE: &[u8] = include_bytes!("../../../firmware/main/assets/focus_done.pcm");
static BREAK_DONE: &[u8] = include_bytes!("../../../firmware/main/assets/break_done.pcm");

fn builtin(prompt: Prompt) -> &'static [u8] {
    match prompt {
        Prompt::InputRequired => INPUT_REQUIRED,
        Prompt::Done => DONE,
        Prompt::Failed => FAILED,
        Prompt::FocusDone => FOCUS_DONE,
        Prompt::BreakDone => BREAK_DONE,
    }
}

enum Source {
    Builtin(&'static [u8]),
    Flash { offset: u32, length: u32 },
}

struct Cursor {
    source: Source,
    position: usize,
}

impl Cursor {
    fn new(command: &PlayCommand) -> Self {
        let source = match command.clips {
            Some(table) => {
                let clip = table[clip_index(command.prompt)];
                Source::Flash { offset: clip.offset, length: clip.length }
            }
            None => Source::Builtin(builtin(command.prompt)),
        };
        Self { source, position: 0 }
    }

    fn remaining(&self) -> usize {
        let total = match self.source {
            Source::Builtin(data) => data.len(),
            Source::Flash { length, .. } => length as usize,
        };
        total - self.position
    }

    /// 往 `out` 里填下一段，返回填了多少。flash 读失败就当这一句结束。
    fn fill(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.remaining());
        let ok = match self.source {
            Source::Builtin(data) => {
                out[..count].copy_from_slice(&data[self.position..self.position + count]);
                true
            }
            Source::Flash { offset, .. } => storage::read_unaligned(offset + self.position as u32, &mut out[..count]).is_ok(),
        };
        if !ok {
            self.position += self.remaining();
            return 0;
        }
        self.position += count;
        count
    }
}

#[embassy_executor::task]
pub async fn audio_task(tx: I2sTx<'static, Async>, buffer: DmaTxStreamBuf) {
    let mut idle_tx = Some(tx);
    let mut idle_buffer = Some(buffer);
    let mut current: Option<Cursor> = None;
    loop {
        // 开流（开机，或者上一次流因为主循环卡太久而断了）。缓冲先清零，
        // 断流前没放完的那一截不能再放一遍。
        let (tx, buffer) = (idle_tx.take().unwrap(), idle_buffer.take().unwrap());
        let (descriptors, mut bytes) = buffer.split();
        bytes.fill(0);
        let buffer = DmaTxStreamBuf::new(descriptors, bytes).unwrap();
        let mut transfer = match tx.write(buffer) {
            Ok(transfer) => transfer,
            Err((_, tx, buffer)) => {
                idle_tx = Some(tx);
                idle_buffer = Some(buffer);
                Timer::after_millis(100).await;
                continue;
            }
        };

        // 连续几轮看到 I2S 空闲。
        let mut idle_rounds = 0;
        loop {
            // 先看流断了没有：I2S 空闲（tx_stop_en 让它在 FIFO 放空时停时钟）
            // 连续两轮、隔 10 ms 都成立才算。只看一次不够，刚开流那一瞬它也是
            // 空闲；也不能要求描述符全部回到 CPU 手里——DMA 在处理最后一块时
            // 就读走了它的 next，之后挂上去的块永远不会被取走、也永远不归还。
            idle_rounds = if transfer.is_done() { idle_rounds + 1 } else { 0 };
            if idle_rounds >= 2 {
                let (tx, buffer) = transfer.stop();
                idle_tx = Some(tx);
                idle_buffer = Some(buffer);
                break;
            }
            if STOP.swap(false, Ordering::AcqRel) {
                current = None;
            }

            // 按整块推：一块正好对应一个 DMA 描述符，句子的尾巴用静音补齐。
            // 半块不交出去，流里就不会留一个 DMA 等不到的描述符。
            let mut chunk = [0u8; STREAM_CHUNK];
            while transfer.available_bytes() >= STREAM_CHUNK {
                if current.is_none()
                    && let Ok(command) = QUEUE.try_receive()
                {
                    current = Some(Cursor::new(&command));
                }
                PLAYING.store(current.is_some(), Ordering::Release);
                let mut filled = 0;
                if let Some(cursor) = current.as_mut() {
                    filled = cursor.fill(&mut chunk);
                    if cursor.remaining() == 0 {
                        current = None;
                    }
                }
                chunk[filled..].fill(0);
                transfer.push(&chunk);
            }
            PLAYING.store(current.is_some(), Ordering::Release);

            Timer::after_millis(10).await;
        }
    }
}
