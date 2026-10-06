//! Voice playback task: I2S outputs continuously from a DMA stream buffer, playing a line when
//! there is one and silence otherwise.
//!
//! The C firmware runs I2S with auto_clear, so the clock never stops and zeros go out when there is
//! no data; the ES8311 uses BCLK as its clock source, and a clock that stops and restarts tends to
//! pop. Same here: the stream stays open and pushes zeros while idle. When the main loop stalls
//! (writing flash, refreshing the screen) the stream has about 170 ms of headroom; a longer stall
//! (erasing the voices partition takes several seconds) breaks the stream, after which the buffer is
//! zeroed and it starts over.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use esp_hal::Async;
use esp_hal::dma::DmaTxStreamBuf;
use esp_hal::i2s::master::I2sTx;
use vibebuddy_firmware_core::adpcm::LineStream;
use vibebuddy_firmware_core::audio::{Chime, Codec, Prompt, Sound};

use crate::storage;

/// Stream buffer: 24 kHz × stereo × 16 bit = 96 KB/s, so 16 KB is about 170 ms.
pub const STREAM_BYTES: usize = 16000;
pub const STREAM_CHUNK: usize = 1000;

pub struct PlayCommand {
    pub sound: Sound,
}

/// As in the C firmware, at most 8 lines are queued.
pub static QUEUE: Channel<CriticalSectionRawMutex, PlayCommand, 8> = Channel::new();
/// A line is playing (from leaving the queue until its last byte is pushed into the buffer).
pub static PLAYING: AtomicBool = AtomicBool::new(false);
/// The main loop asks for an immediate stop: drop what is playing; the requester clears the queue.
pub static STOP: AtomicBool = AtomicBool::new(false);

static INPUT_REQUIRED: &[u8] = include_bytes!("../../../firmware/main/assets/input_required.pcm");
static DONE: &[u8] = include_bytes!("../../../firmware/main/assets/done.pcm");
static FAILED: &[u8] = include_bytes!("../../../firmware/main/assets/failed.pcm");
static FOCUS_DONE: &[u8] = include_bytes!("../../../firmware/main/assets/focus_done.pcm");
static BREAK_DONE: &[u8] = include_bytes!("../../../firmware/main/assets/break_done.pcm");
static FOCUS_CHIME: &[u8] = include_bytes!("../../../firmware/main/assets/focus_chime.pcm");
static BREAK_CHIME: &[u8] = include_bytes!("../../../firmware/main/assets/break_chime.pcm");

fn builtin(prompt: Prompt) -> &'static [u8] {
    match prompt {
        Prompt::InputRequired => INPUT_REQUIRED,
        Prompt::Done => DONE,
        Prompt::Failed => FAILED,
        Prompt::FocusDone => FOCUS_DONE,
        Prompt::BreakDone => BREAK_DONE,
    }
}

/// Compressed bytes read from flash ahead of the decoder.
struct Pending {
    offset: u32,
    left: u32,
    buffer: [u8; 128],
    length: usize,
    position: usize,
    failed: bool,
}

impl Pending {
    fn next_byte(&mut self) -> u8 {
        if self.position == self.length {
            let count = (self.left as usize).min(self.buffer.len());
            if count == 0 || storage::read_unaligned(self.offset, &mut self.buffer[..count]).is_err() {
                self.failed = true;
                return 0;
            }
            self.offset += count as u32;
            self.left -= count as u32;
            self.length = count;
            self.position = 0;
        }
        self.position += 1;
        self.buffer[self.position - 1]
    }
}

enum Source {
    Builtin(&'static [u8]),
    Flash { offset: u32, length: u32 },
    /// A Character's line: decoded and upsampled on the way out.
    Adpcm { stream: LineStream, pending: Pending },
}

struct Cursor {
    source: Source,
    position: usize,
}

impl Cursor {
    fn new(command: &PlayCommand) -> Self {
        let source = match command.sound {
            Sound::Builtin(prompt) => Source::Builtin(builtin(prompt)),
            Sound::Chime(Chime::Focus) => Source::Builtin(FOCUS_CHIME),
            Sound::Chime(Chime::Break) => Source::Builtin(BREAK_CHIME),
            Sound::Line(line) => match line.codec {
                Codec::Pcm24kStereo => Source::Flash { offset: line.offset, length: line.length },
                Codec::Adpcm16kMono { samples } => Source::Adpcm {
                    stream: LineStream::new(samples),
                    pending: Pending { offset: line.offset, left: line.length, buffer: [0; 128], length: 0, position: 0, failed: false },
                },
            },
        };
        Self { source, position: 0 }
    }

    fn remaining(&self) -> usize {
        let total = match &self.source {
            Source::Builtin(data) => data.len(),
            Source::Flash { length, .. } => *length as usize,
            Source::Adpcm { stream, .. } => return stream.remaining_bytes(),
        };
        total - self.position
    }

    /// Fills `out` with the next piece and returns how much was filled. A failed flash read ends the
    /// line.
    fn fill(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.remaining());
        let ok = match &mut self.source {
            Source::Builtin(data) => {
                out[..count].copy_from_slice(&data[self.position..self.position + count]);
                true
            }
            Source::Flash { offset, .. } => storage::read_unaligned(*offset + self.position as u32, &mut out[..count]).is_ok(),
            Source::Adpcm { stream, pending } => {
                let written = stream.fill(&mut out[..count / 4 * 4], || pending.next_byte());
                if pending.failed {
                    *stream = LineStream::new(0);
                    return 0;
                }
                return written;
            }
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
        // Open the stream (at boot, or after the last one broke because the main loop stalled too
        // long). Zero the buffer first: the unplayed part from before the break must not play again.
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

        // Consecutive rounds that saw I2S idle.
        let mut idle_rounds = 0;
        loop {
            // First check whether the stream broke: it counts only if I2S is idle (tx_stop_en stops
            // the clock once the FIFO drains) for two rounds in a row, 10 ms apart. One look is not
            // enough, since it is also idle right after the stream opens; nor can we require every
            // descriptor to be back with the CPU: DMA reads the last block's next pointer while
            // processing it, so a block linked on afterwards is never fetched and never returned.
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

            // Push whole chunks: one chunk maps to exactly one DMA descriptor, and a line's tail is
            // padded with silence. Never handing over half a chunk means the stream never holds a
            // descriptor DMA will not reach.
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
