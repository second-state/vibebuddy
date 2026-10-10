//! A stand-in when there is no box: firmware-core plus an in-memory fake board, with the serial port
//! replaced by stdin/stdout. `tools/firmware-smoke.py` can run against it to validate the script and the
//! whole serial chain first.
//!
//! Usage: python3 tools/simulate-device.py (it opens a pseudo-terminal and attaches this program to it).
//! `--tcp <port>` also listens on the local network side, as a box on Wi-Fi does (docs/protocol.md,
//! "Local network link"); pair over the pseudo-terminal first, as a box is paired over USB.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use vibebuddy_firmware_core::audio::Sound;
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::{Flash, FlashError};

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
    /// Network connections by number, and the one linked.
    conns: HashMap<u32, TcpStream>,
    peer: Option<u32>,
}

/// What the main loop hears from the stdin and network threads.
enum Input {
    Cable(Vec<u8>),
    Opened(u32, TcpStream),
    Received(u32, Vec<u8>),
    Closed(u32),
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
        if let Some(stream) = self.peer.and_then(|conn| self.conns.get_mut(&conn)) {
            let _ = stream.write_all(bytes);
        }
    }
    fn lan_send(&mut self, conn: u32, bytes: &[u8]) {
        if let Some(stream) = self.conns.get_mut(&conn) {
            let _ = stream.write_all(bytes);
        }
    }
    fn lan_close(&mut self, conn: u32) {
        if let Some(stream) = self.conns.remove(&conn) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    fn lan_peer(&mut self, conn: Option<u32>) {
        self.peer = conn;
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
}

/// 16 MB flash, with the partition table laid out per firmware/partitions.csv.
fn flash_with_partitions() -> MemoryFlash {
    let mut bytes = vec![0xFF; 0xA14000];
    let entries = [
        ("nvs", 1u8, 2u8, 0x9000u32, 0x6000u32),
        ("phy_init", 1, 1, 0xF000, 0x1000),
        ("ota_0", 0, 0x10, 0x10000, 0x400000),
        ("voices", 1, 0x40, 0x410000, 0x200000),
        ("ota_1", 0, 0x11, 0x610000, 0x400000),
        ("otadata", 1, 0, 0xA10000, 0x2000),
        ("link", 1, 0x41, 0xA12000, 0x2000),
    ];
    for (index, (label, kind, subtype, offset, size)) in entries.iter().enumerate() {
        let entry = &mut bytes[0x8000 + index * 32..0x8000 + index * 32 + 32];
        entry.fill(0);
        entry[..4].copy_from_slice(&[0xAA, 0x50, *kind, *subtype]);
        entry[4..8].copy_from_slice(&offset.to_le_bytes());
        entry[8..12].copy_from_slice(&size.to_le_bytes());
        entry[12..12 + label.len()].copy_from_slice(label.as_bytes());
    }
    MemoryFlash(bytes)
}

/// Accepts connections on `port`, numbering them, and reads each on its own thread.
fn listen(port: u16, sender: mpsc::Sender<Input>) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|error| panic!("can't listen on {port}: {error}"));
    std::thread::spawn(move || {
        for (conn, stream) in (1u32..).zip(listener.incoming().flatten()) {
            let Ok(mut reader) = stream.try_clone() else { continue };
            if sender.send(Input::Opened(conn, stream)).is_err() {
                break;
            }
            let sender = sender.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 256];
                while let Ok(count @ 1..) = reader.read(&mut chunk) {
                    if sender.send(Input::Received(conn, chunk[..count].to_vec())).is_err() {
                        return;
                    }
                }
                let _ = sender.send(Input::Closed(conn));
            });
        }
    });
}

fn main() {
    let (sender, receiver) = mpsc::channel::<Input>();
    let cable = sender.clone();
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        let mut chunk = [0u8; 256];
        while let Ok(count) = input.read(&mut chunk) {
            if count == 0 || cable.send(Input::Cable(chunk[..count].to_vec())).is_err() {
                break;
            }
        }
    });
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(port) = arguments.iter().position(|argument| argument == "--tcp").and_then(|index| arguments.get(index + 1)) {
        listen(port.parse().expect("--tcp takes a port"), sender);
    }
    let mut board = SimulatedBoard {
        start: Instant::now(),
        flash: flash_with_partitions(),
        frame: vec![0; FRAME_BYTES],
        out: std::io::stdout(),
        conns: HashMap::new(),
        peer: None,
    };
    let build = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../device/build/build.txt")).unwrap_or_else(|_| "simulator".to_owned());
    let mut firmware = Firmware::new(1, board.now_ms(), build.trim().as_bytes(), "0.0.0");
    firmware.boot(&mut board);
    loop {
        while let Ok(input) = receiver.try_recv() {
            match input {
                Input::Cable(bytes) => firmware.receive(&mut board, &bytes),
                Input::Opened(conn, stream) => {
                    board.conns.insert(conn, stream);
                    // Not secret here: the simulator's key is fixed anyway.
                    let nonce: [u8; 32] = std::array::from_fn(|index| (board.now_ms() as usize + index * 31 + conn as usize) as u8);
                    firmware.lan_opened(&mut board, conn, nonce);
                }
                Input::Received(conn, bytes) => firmware.lan_received(&mut board, conn, &bytes),
                Input::Closed(conn) => {
                    board.conns.remove(&conn);
                    firmware.lan_closed(&mut board, conn);
                }
            }
        }
        firmware.poll(&mut board);
        std::thread::sleep(Duration::from_millis(10));
    }
}
