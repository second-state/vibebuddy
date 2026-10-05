//! Renders the device menu (docs/device-menu.md) to PPM on the Mac: the real firmware on a fake
//! board, driven by key presses, so the pictures in the docs are what the box shows.
//!
//! Usage: cargo run -p vibebuddy-firmware-core --example menu_preview -- <directory>

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::audio::Prompt;
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::{Flash, FlashError};
use vibebuddy_firmware_core::voices::ClipTable;

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
    other_app: bool,
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
    fn play(&mut self, _prompt: Prompt, _clips: ClipTable) -> Result<(), ()> {
        Ok(())
    }
    fn stop_audio(&mut self) {}
    fn audio_busy(&self) -> bool {
        false
    }
    fn set_volume(&mut self, _level: u32) -> Result<(), VolumeError> {
        Ok(())
    }
    fn has_other_app(&mut self) -> bool {
        self.other_app
    }
}

struct Preview {
    directory: PathBuf,
    board: PreviewBoard,
    firmware: Firmware,
}

impl Preview {
    fn new(directory: &Path, other_app: bool) -> Self {
        let mut board = PreviewBoard { now: Cell::new(1000), frame: vec![0; FRAME_BYTES], flash: NoFlash, keys: (false, false, false), other_app };
        let mut firmware = Firmware::new(1, 1000, b"v0.3.0 2026-10-05 16:17");
        firmware.boot(&mut board);
        let mut preview = Self { directory: directory.to_path_buf(), board, firmware };
        preview.send(r#"{"version":1,"event":"device.heartbeat","build":"v0.3.0 2026-10-05 11:11","hour":14,"day":20261005}"#);
        preview.send(
            r#"{"version":1,"event":"task.start","title":"CC:MUSE DUAL BOOT","stats":["13 DONE","1 ASKS","46M BUSY"],"tasks":[{"title":"CC:MUSE DUAL BOOT","status":"working","elapsed_s":420,"project":"VIBE-BUDDY"},{"title":"CX:RELEASE NOTES","status":"done","elapsed_s":1500,"project":"VIBE-BUDDY"}]}"#,
        );
        preview
    }

    fn send(&mut self, line: &str) {
        self.firmware.receive(&mut self.board, line.as_bytes());
        self.firmware.receive(&mut self.board, b"\n");
    }

    fn press(&mut self, key: usize, hold_ms: u32) {
        let set = |board: &mut PreviewBoard, down: bool| match key {
            0 => board.keys.0 = down,
            1 => board.keys.1 = down,
            _ => board.keys.2 = down,
        };
        self.board.now.set(self.board.now.get() + 100);
        set(&mut self.board, true);
        self.firmware.poll(&mut self.board);
        let mut held = 0;
        while held < hold_ms {
            self.board.now.set(self.board.now.get() + 50);
            held += 50;
            self.firmware.poll(&mut self.board);
        }
        set(&mut self.board, false);
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
    let directory = PathBuf::from(std::env::args().nth(1).expect("usage: menu_preview <output directory>"));
    fs::create_dir_all(&directory).expect("create directory");

    let mut preview = Preview::new(&directory, false);
    preview.press(1, 1100);
    preview.snapshot("menu-open");
    preview.press(0, 50);
    preview.snapshot("menu-volume");
    for _ in 0..2 {
        preview.press(1, 50);
    }
    preview.press(0, 50);
    preview.snapshot("menu-status");

    let mut preview = Preview::new(&directory, true);
    preview.press(0, 50);
    preview.press(1, 1100);
    preview.snapshot("menu-focus");
    for _ in 0..3 {
        preview.press(1, 50);
    }
    preview.press(0, 50);
    preview.snapshot("menu-muse");
}
