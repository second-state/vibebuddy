//! A stand-in when there is no box: firmware-core plus an in-memory fake board, with the serial port
//! replaced by stdin/stdout. `tools/firmware-smoke.py` can run against it to validate the script and the
//! whole serial chain first.
//!
//! Usage: python3 tools/simulate-device.py (it opens a pseudo-terminal and attaches this program to it).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use vibebuddy_firmware_core::audio::Prompt;
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::{Flash, FlashError};
use vibebuddy_firmware_core::voices::ClipTable;

struct MemoryFlash(Vec<u8>);

impl Flash for MemoryFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        let start = offset as usize;
        bytes.copy_from_slice(self.0.get(start..start + bytes.len()).ok_or(FlashError)?);
        Ok(())
    }
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        let start = offset as usize;
        for (cell, byte) in self.0.get_mut(start..start + bytes.len()).ok_or(FlashError)?.iter_mut().zip(bytes) {
            *cell &= byte;
        }
        Ok(())
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        self.0.get_mut(from as usize..to as usize).ok_or(FlashError)?.fill(0xFF);
        Ok(())
    }
}

struct SimulatedBoard {
    start: Instant,
    flash: MemoryFlash,
    frame: Vec<u8>,
    out: std::io::Stdout,
}

impl Screen for SimulatedBoard {
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

impl Board for SimulatedBoard {
    fn now_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }
    fn flash(&mut self) -> &mut dyn Flash {
        &mut self.flash
    }
    fn with_frame_and_output(&mut self, action: &mut FrameAction) {
        let frame = &self.frame;
        let out = &mut self.out;
        action(frame, &mut |bytes| {
            let _ = out.write_all(bytes);
            let _ = out.flush();
        });
    }
    fn init_display(&mut self) -> bool {
        true
    }
    fn init_audio(&mut self, _volume: u32) -> AudioStatus {
        Ok("SIMULATED")
    }
    fn init_buttons(&mut self) -> Option<Levels> {
        Some(Levels::default())
    }
    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>) {
        (false, Some((false, false)))
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
}

/// 16 MB flash, with the partition table laid out per firmware/partitions.csv.
fn flash_with_partitions() -> MemoryFlash {
    let mut bytes = vec![0xFF; 0x610000];
    let entries = [("nvs", 0x9000u32, 0x6000u32), ("phy_init", 0xF000, 0x1000), ("factory", 0x10000, 0x400000), ("voices", 0x410000, 0x200000)];
    for (index, (label, offset, size)) in entries.iter().enumerate() {
        let entry = &mut bytes[0x8000 + index * 32..0x8000 + index * 32 + 32];
        entry.fill(0);
        entry[..2].copy_from_slice(&[0xAA, 0x50]);
        entry[4..8].copy_from_slice(&offset.to_le_bytes());
        entry[8..12].copy_from_slice(&size.to_le_bytes());
        entry[12..12 + label.len()].copy_from_slice(label.as_bytes());
    }
    MemoryFlash(bytes)
}

fn main() {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let mut chunk = [0u8; 256];
        while let Ok(count) = input.read(&mut chunk) {
            if count == 0 || sender.send(chunk[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut board = SimulatedBoard { start: Instant::now(), flash: flash_with_partitions(), frame: vec![0; FRAME_BYTES], out: std::io::stdout() };
    let build = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../device/build/build.txt")).unwrap_or_else(|_| "simulator".to_owned());
    let mut firmware = Firmware::new(1, board.now_ms(), build.trim().as_bytes());
    firmware.boot(&mut board);
    loop {
        while let Ok(bytes) = receiver.try_recv() {
            firmware.receive(&mut board, &bytes);
        }
        firmware.poll(&mut board);
        std::thread::sleep(Duration::from_millis(10));
    }
}
