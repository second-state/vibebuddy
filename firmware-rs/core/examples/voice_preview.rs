//! Renders a voice command's overlay (docs/device-menu.md, "Voice commands") to PPM on the Mac: the real
//! firmware on a fake board, with a fake microphone and the computer's answers fed in as lines, so the
//! pictures in the docs are what the box shows. Also the Chinese text pictures (docs/firmware-bringup.md,
//! "Fonts").
//!
//! Usage: cargo run -p vibebuddy-firmware-core --example voice_preview -- <directory>

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::audio::Sound;
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::{Flash, FlashError};

/// No partitions: settings and voice packs aren't what these pictures are about.
struct NoFlash;

impl Flash for NoFlash {
    fn read(&mut self, _offset: u32, _bytes: &mut [u8]) -> Result<(), FlashError> {
        Err(FlashError)
    }
    fn write(&mut self, _offset: u32, _bytes: &[u8]) -> Result<(), FlashError> {
        Err(FlashError)
    }
    fn erase(&mut self, _from: u32, _to: u32) -> Result<(), FlashError> {
        Err(FlashError)
    }
}

struct PreviewBoard {
    now: Cell<u32>,
    frame: Vec<u8>,
    flash: NoFlash,
    keys: (bool, bool, bool),
    /// The fake microphone: running, and what it has recorded since the last take.
    mic: bool,
    samples: Vec<i16>,
}

impl Screen for PreviewBoard {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.frame
    }
    fn present(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn set_backlight(&mut self, _on: bool) -> Result<(), ()> {
        Ok(())
    }
}

impl Board for PreviewBoard {
    fn now_ms(&self) -> u32 {
        self.now.get()
    }
    fn write(&mut self, _bytes: &[u8]) {}
    fn flash(&mut self) -> &mut dyn Flash {
        &mut self.flash
    }
    fn with_frame_and_output(&mut self, _action: &mut FrameAction) {}
    fn init_display(&mut self) -> bool {
        true
    }
    fn init_audio(&mut self, _volume: u32) -> AudioStatus {
        Ok("ES8311")
    }
    fn init_buttons(&mut self) -> Option<Levels> {
        Some(Levels::default())
    }
    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>) {
        (self.keys.0, Some((self.keys.1, self.keys.2)))
    }
    fn play(&mut self, _sound: Sound) -> Result<(), ()> {
        Ok(())
    }
    fn stop_audio(&mut self) {}
    fn audio_busy(&self) -> bool {
        false
    }
    fn set_volume(&mut self, _level: u32) -> Result<(), VolumeError> {
        Ok(())
    }
    fn voice_start(&mut self) -> Result<(), ()> {
        self.mic = true;
        Ok(())
    }
    fn voice_take(&mut self, samples: &mut Vec<i16>) {
        samples.append(&mut self.samples);
    }
    fn voice_stop(&mut self) {
        self.mic = false;
    }
}

struct Preview {
    directory: PathBuf,
    board: PreviewBoard,
    firmware: Firmware,
}

impl Preview {
    fn new(directory: &Path) -> Self {
        let mut board = PreviewBoard { now: Cell::new(1000), frame: vec![0; FRAME_BYTES], flash: NoFlash, keys: (false, false, false), mic: false, samples: Vec::new() };
        let mut firmware = Firmware::new(1, 1000, b"v0.3.2-11-g0adc18b 2026-10-06 16:34", "0.3.2");
        firmware.boot(&mut board);
        let mut preview = Self { directory: directory.to_path_buf(), board, firmware };
        preview.send(r#"{"version":1,"event":"device.heartbeat","build":"0.3.1 v0.3.1-12-ga4a667c 2026-10-06 16:26","hour":14,"day":20261005}"#);
        preview.send(
            r#"{"version":1,"event":"task.start","title":"CC:MUSE DUAL BOOT","stats":["13 DONE","1 ASKS","46M BUSY"],"tasks":[{"title":"CC:MUSE DUAL BOOT","status":"working","elapsed_s":420,"project":"VIBE-BUDDY"},{"title":"CX:RELEASE NOTES","status":"done","elapsed_s":1500,"project":"VIBE-BUDDY"}]}"#,
        );
        preview
    }

    fn send(&mut self, line: &str) {
        self.firmware.receive(&mut self.board, line.as_bytes());
        self.firmware.receive(&mut self.board, b"\n");
    }

    /// Holds K0 for `hold_ms`; while the microphone runs it hears a tone at `amplitude`.
    fn hold_k0(&mut self, hold_ms: u32, amplitude: f32) {
        self.board.keys.0 = true;
        let mut held = 0;
        while held < hold_ms {
            self.board.now.set(self.board.now.get() + 10);
            held += 10;
            if self.board.mic {
                let start = self.board.now.get() as f32 * 24.0;
                self.board.samples.extend((0..240).map(|index| ((start + index as f32) * 0.06).sin() * amplitude).map(|sample| sample as i16));
            }
            self.firmware.poll(&mut self.board);
        }
    }

    fn release_k0(&mut self) {
        self.board.keys.0 = false;
        self.board.now.set(self.board.now.get() + 10);
        self.firmware.poll(&mut self.board);
    }

    fn snapshot(&self, name: &str) {
        let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        for pair in self.board.frame[..FRAME_BYTES].as_chunks::<2>().0 {
            let pixel = u16::from_be_bytes([pair[0], pair[1]]) as u32;
            out.push((((pixel >> 11) & 0x1f) * 255 / 31) as u8);
            out.push((((pixel >> 5) & 0x3f) * 255 / 63) as u8);
            out.push(((pixel & 0x1f) * 255 / 31) as u8);
        }
        fs::write(self.directory.join(format!("{name}.ppm")), out).expect("write PPM");
    }
}

fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("usage: voice_preview <output directory>"));
    fs::create_dir_all(&directory).expect("create directory");

    let mut preview = Preview::new(&directory);
    preview.send(r#"{"version":1,"event":"voice.available","on":true}"#);
    // Four seconds in (one of them the long press itself), speaking at about -21 dBFS.
    preview.hold_k0(4000, 4000.0);
    preview.snapshot("voice-listening");
    preview.release_k0();
    preview.snapshot("voice-thinking");
    preview.send(r#"{"version":1,"event":"voice.heard","turn":1,"text":"what is codex doing right now"}"#);
    preview.send(r#"{"version":1,"event":"voice.reply","turn":1,"text":"It's writing the release notes. Should be done in a few minutes."}"#);
    preview.snapshot("voice-reply");

    preview.hold_k0(2500, 4000.0);
    preview.release_k0();
    preview.send(r#"{"version":1,"event":"voice.error","turn":2,"reason":"not_heard"}"#);
    preview.snapshot("voice-error");

    // Chinese (font.rs): session titles on the task cards, then a voice turn in large and in small type.
    let mut preview = Preview::new(&directory);
    preview.send(
        r#"{"version":1,"event":"task.start","title":"CC:给盒子加中文字体","tasks":[{"title":"CC:给盒子加中文字体","status":"working","elapsed_s":420,"project":"VIBE-BUDDY"},{"title":"CX:修好语音回复的播放","status":"input_required","elapsed_s":95,"project":"VIBE-BUDDY"},{"title":"CC:整理发布说明","status":"done","elapsed_s":1500,"project":"官网"}]}"#,
    );
    preview.snapshot("device-cards-zh");
    preview.send(r#"{"version":1,"event":"voice.available","on":true}"#);
    preview.hold_k0(2500, 4000.0);
    preview.release_k0();
    preview.send(r#"{"version":1,"event":"voice.heard","turn":1,"text":"Codex 现在在忙什么？"}"#);
    preview.send(r#"{"version":1,"event":"voice.reply","turn":1,"text":"它在写 v0.5.1 的发布说明，大概还要五分钟。写完我提醒你。"}"#);
    preview.snapshot("voice-reply-zh");
    preview.send(
        r#"{"version":1,"event":"voice.reply","turn":1,"text":"另外，Claude Code 那边在等你确认：要不要把字体表放进 flash 的只读数据里？它估计会多占七十多 KB，但完全不占内存。你说“可以”，我就让它继续。"}"#,
    );
    preview.snapshot("voice-reply-zh-small");
}
