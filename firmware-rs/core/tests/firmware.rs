//! Host tests for the whole chain: a fake board, JSON fed over serial, then check what came
//! back, what was stored and what was played. The expected output lines are taken from the C
//! firmware's behavior, and the Mac's parsing relies on them staying verbatim.

use std::cell::Cell;

use vibebuddy_firmware_core::audio::{Codec, Prompt, Sound};
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError, hold_line_byte};
use vibebuddy_firmware_core::noise::Sealer;
use vibebuddy_firmware_core::spoken_reply::{ReplyAudio, ReplyPlayer};
use vibebuddy_firmware_core::storage::{Flash, FlashError};
use vibebuddy_firmware_core::touch;
use vibebuddy_firmware_core::voice_pack;

struct MemoryFlash {
    bytes: Vec<u8>,
}

impl Flash for MemoryFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        let start = offset as usize;
        bytes.copy_from_slice(self.bytes.get(start..start + bytes.len()).ok_or(FlashError)?);
        Ok(())
    }
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        assert_eq!((offset % 4, bytes.len() % 4), (0, 0), "flash write is not aligned");
        let start = offset as usize;
        for (cell, &byte) in self.bytes.get_mut(start..start + bytes.len()).ok_or(FlashError)?.iter_mut().zip(bytes) {
            *cell &= byte;
        }
        Ok(())
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        self.bytes.get_mut(from as usize..to as usize).ok_or(FlashError)?.fill(0xFF);
        Ok(())
    }
}

/// 16 MB flash, with a partition table matching partitions.csv.
fn blank_flash() -> MemoryFlash {
    let mut flash = MemoryFlash { bytes: vec![0xFF; 0xA14000] };
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
        let at = 0x8000 + index * 32;
        let entry = &mut flash.bytes[at..at + 32];
        entry.fill(0);
        entry[0] = 0xAA;
        entry[1] = 0x50;
        entry[2] = *kind;
        entry[3] = *subtype;
        entry[4..8].copy_from_slice(&offset.to_le_bytes());
        entry[8..12].copy_from_slice(&size.to_le_bytes());
        entry[12..12 + label.len()].copy_from_slice(label.as_bytes());
    }
    flash
}

struct FakeBoard {
    now: Cell<u32>,
    output: Vec<u8>,
    flash: MemoryFlash,
    frame: Vec<u8>,
    presents: usize,
    backlight: bool,
    codec_volume: Option<u32>,
    played: Vec<Sound>,
    busy: bool,
    stops: usize,
    k0: bool,
    k1: bool,
    k2: bool,
    /// What went to each network connection, which ones were closed, and the linked one.
    lan_sent: Vec<(u32, String)>,
    lan_closed: Vec<u32>,
    lan_peer: Option<u32>,
    /// What went sealed to the computer linked over the network, and whether one is linked through the Relay.
    lan_peer_sent: Vec<String>,
    relay_peer: bool,
    /// The linked network computer's buffer is full.
    peer_full: bool,
    /// Restarts asked for.
    restarts: usize,
    /// UART rates asked for with `device.baud`.
    uart_rates: Vec<u32>,
    /// Registers of the chips on I2C, by (address, register), and what was written.
    i2c: std::collections::BTreeMap<(u8, u8), u8>,
    i2c_writes: Vec<(u8, u8, u8)>,
    /// The board has I2S input; and the size of the capture asked for, until it is taken.
    mic_input: bool,
    mic_capture: Option<usize>,
    /// The voice command microphone is running; samples waiting to be taken; `VOICE DATA` lines written.
    voice_running: bool,
    voice_samples: Vec<i16>,
    stream: Vec<String>,
    /// What the spoken reply player was handed, and whether it says a reply is sounding.
    reply_audio: Vec<ReplyAudio>,
    speaking: bool,
    /// The box has a touchscreen, and where a finger is on it.
    touchscreen: bool,
    finger: Option<(i32, i32)>,
}

impl FakeBoard {
    fn new(flash: MemoryFlash) -> Self {
        Self {
            now: Cell::new(1000),
            output: Vec::new(),
            flash,
            frame: vec![0; FRAME_BYTES],
            presents: 0,
            backlight: false,
            codec_volume: None,
            played: Vec::new(),
            busy: false,
            stops: 0,
            k0: false,
            k1: false,
            k2: false,
            lan_sent: Vec::new(),
            lan_closed: Vec::new(),
            lan_peer: None,
            lan_peer_sent: Vec::new(),
            relay_peer: false,
            peer_full: false,
            restarts: 0,
            uart_rates: Vec::new(),
            i2c: [((0x18, 0xFD), 0x83), ((0x18, 0xFE), 0x11), ((0x18, 0x16), 0x24)].into(),
            i2c_writes: Vec::new(),
            mic_input: true,
            mic_capture: None,
            voice_running: false,
            voice_samples: Vec::new(),
            stream: Vec::new(),
            reply_audio: Vec::new(),
            speaking: false,
            touchscreen: false,
            finger: None,
        }
    }

    fn take_lines(&mut self) -> Vec<String> {
        let text = String::from_utf8(std::mem::take(&mut self.output)).expect("output is UTF-8");
        text.lines().map(str::to_owned).collect()
    }

    fn advance(&self, ms: u32) {
        self.now.set(self.now.get() + ms);
    }
}

impl Screen for FakeBoard {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.frame
    }
    fn present(&mut self) -> Result<(), ()> {
        self.presents += 1;
        Ok(())
    }
    fn set_backlight(&mut self, on: bool) -> Result<(), ()> {
        self.backlight = on;
        Ok(())
    }
}

impl Board for FakeBoard {
    fn lan_send(&mut self, conn: u32, bytes: &[u8]) {
        self.lan_sent.push((conn, String::from_utf8(bytes.to_vec()).unwrap()));
    }
    fn lan_close(&mut self, conn: u32) {
        self.lan_closed.push(conn);
    }
    fn lan_peer(&mut self, peer: Option<(u32, Sealer)>) {
        self.lan_peer = peer.map(|(conn, _)| conn);
    }
    fn lan_peer_send(&mut self, line: &[u8]) {
        self.lan_peer_sent.push(String::from_utf8(line.to_vec()).unwrap());
    }
    fn relay_peer(&mut self, sealer: Option<Sealer>) {
        self.relay_peer = sealer.is_some();
    }
    fn peer_can_take(&self, _bytes: usize) -> bool {
        !self.peer_full
    }
    fn random(&mut self) -> Option<[u8; 32]> {
        Some([0x5a; 32])
    }
    fn restart(&mut self) {
        self.restarts += 1;
    }
    fn set_uart_rate(&mut self, rate: u32) {
        self.uart_rates.push(rate);
    }
    fn now_ms(&self) -> u32 {
        self.now.get()
    }
    fn write(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }
    fn flash(&mut self) -> &mut dyn Flash {
        &mut self.flash
    }
    fn with_frame_and_output(&mut self, action: &mut FrameAction) {
        let frame = &self.frame;
        let output = &mut self.output;
        action(frame, &mut |bytes| output.extend_from_slice(bytes));
    }
    fn init_display(&mut self) -> bool {
        true
    }
    fn init_audio(&mut self, volume: u32) -> AudioStatus {
        self.codec_volume = Some(volume);
        Ok("ES8311")
    }
    fn init_buttons(&mut self) -> Option<Levels> {
        Some(Levels::default())
    }
    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>) {
        (self.k0, Some((self.k1, self.k2)))
    }
    fn init_touch(&mut self) -> Option<&'static str> {
        self.touchscreen.then_some("CHSC5432")
    }
    /// The report the chip would send, the inverse of `touch::parse_report`.
    fn read_touch(&mut self) -> Option<[u8; touch::REPORT_BYTES]> {
        let mut report = [0; touch::REPORT_BYTES];
        if let Some((x, y)) = self.finger {
            let short = (239 - y) as u16;
            let long = x as u16;
            report[1] = 1;
            report[2] = short as u8;
            report[3] = long as u8;
            report[5] = ((long >> 8) as u8) << 4 | (short >> 8) as u8;
        }
        Some(report)
    }
    fn play(&mut self, sound: Sound) -> Result<(), ()> {
        self.played.push(sound);
        Ok(())
    }
    fn stop_audio(&mut self) {
        self.stops += 1;
    }
    fn audio_busy(&self) -> bool {
        self.busy
    }
    fn set_volume(&mut self, level: u32) -> Result<(), VolumeError> {
        self.codec_volume = Some(level);
        Ok(())
    }
    fn i2c_read(&mut self, address: u8, register: u8) -> Option<u8> {
        self.i2c.get(&(address, register)).copied()
    }
    fn i2c_write(&mut self, address: u8, register: u8, value: u8) -> Result<(), ()> {
        self.i2c.keys().any(|&(chip, _)| chip == address).then_some(()).ok_or(())?;
        self.i2c.insert((address, register), value);
        self.i2c_writes.push((address, register, value));
        Ok(())
    }
    fn mic_start(&mut self, bytes: usize) -> Result<(), ()> {
        self.mic_input.then_some(()).ok_or(())?;
        self.mic_capture = Some(bytes);
        Ok(())
    }
    fn voice_start(&mut self) -> Result<(), ()> {
        self.mic_input.then_some(()).ok_or(())?;
        self.voice_running = true;
        Ok(())
    }
    fn voice_take(&mut self, samples: &mut Vec<i16>) {
        samples.append(&mut self.voice_samples);
    }
    fn voice_stop(&mut self) {
        self.voice_running = false;
        self.voice_samples.clear();
    }
    fn write_stream(&mut self, line: &[u8]) {
        self.stream.push(String::from_utf8(line.to_vec()).unwrap());
    }
    fn reply_audio(&mut self, audio: ReplyAudio) {
        self.reply_audio.push(audio);
    }
    fn reply_speaking(&self) -> bool {
        self.speaking
    }
    /// The left slot counts up, the right slot is silent.
    fn mic_take(&mut self) -> Option<Result<Vec<u8>, &'static str>> {
        let bytes = self.mic_capture.take()?;
        Some(Ok((0..bytes / 4).flat_map(|frame| [(frame % 256) as u8, 0, 0, 0]).collect()))
    }
}

/// The key the box makes from the entropy `Firmware::new(7, …)` derives.
const BOX_KEY: &str = "BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE";

const RELAY_URL: &str = "RELAY URL wss://relay.korekore.ai";

fn booted(flash: MemoryFlash) -> (Firmware, FakeBoard, Vec<String>) {
    let mut board = FakeBoard::new(flash);
    let mut firmware = Firmware::new(7, board.now_ms(), b"abc1234 2026-09-26 10:00", "1.2.3");
    firmware.boot(&mut board);
    let lines = board.take_lines();
    (firmware, board, lines)
}

fn send(firmware: &mut Firmware, board: &mut FakeBoard, line: &str) -> Vec<String> {
    firmware.receive(board, line.as_bytes());
    firmware.receive(board, b"\n");
    board.take_lines()
}

#[test]
fn boot_reports_like_the_c_firmware() {
    let (_, board, lines) = booted(blank_flash());
    assert_eq!(
        lines,
        [
            "TALLY LOADED 0 0S DAY 0",
            "DISPLAY READY BUILD abc1234 2026-09-26 10:00",
            "FIRMWARE VERSION 1.2.3",
            "TRANSFER 1",
            "LINK NOISE",
            BOX_KEY,
            RELAY_URL,
            "VOICES builtin",
            "AUDIO READY",
            "AUDIO CODEC ES8311",
            "VOLUME 65",
            "BUTTONS READY",
            "TOUCH NONE",
            "READY vibebuddy-fw 1.2.3",
        ]
    );
    assert!(board.backlight);
    assert!(board.presents > 0);
    assert_eq!(board.codec_volume, Some(65));
}

#[test]
fn a_done_event_is_shown_announced_and_returns_to_idle() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(
        &mut firmware,
        &mut board,
        r#"{"version":1,"event":"task.done","title":"CC:VIBE","tasks":[{"title":"CC:VIBE","status":"done","elapsed_s":3}],"stats":["7 DONE"]}"#,
    );
    assert_eq!(lines, ["EVENT task.done", "TITLE CC:VIBE", "DISPLAY STATE DONE", "AUDIO QUEUED DONE"]);
    assert_eq!(board.played, [Sound::Builtin(Prompt::Done)]);

    let presents = board.presents;
    board.advance(4990);
    firmware.poll(&mut board);
    board.advance(20);
    firmware.poll(&mut board);
    assert!(board.presents > presents, "should redraw as idle after 5 seconds");
    assert_eq!(board.take_lines(), ["DISPLAY STATE READY"]);
}

#[test]
fn after_done_the_device_returns_to_the_tasks_still_running() {
    // A subagent finished while the main session keeps working. The main session may then generate
    // for minutes without a single hook, so nothing would refresh the screen back to working.
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(
        &mut firmware,
        &mut board,
        r#"{"version":1,"event":"task.done","title":"CC:SUB","tasks":[{"title":"CC:MAIN","status":"working","elapsed_s":60}],"announcement":"done"}"#,
    );

    board.advance(5000);
    firmware.poll(&mut board);
    assert_eq!(board.take_lines(), ["DISPLAY STATE WORKING"]);
}

#[test]
fn suppressed_and_announced_audio() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.input_required","suppress_audio":true}"#);
    assert_eq!(lines, ["EVENT agent.input_required", "DISPLAY STATE INPUT REQUIRED"]);
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.idle","announcement":"failed"}"#);
    assert_eq!(lines, ["EVENT agent.idle", "DISPLAY STATE READY", "AUDIO QUEUED FAILED"]);
}

fn last_audio(firmware: &mut Firmware, board: &mut FakeBoard, line: &str) -> String {
    send(firmware, board, line).into_iter().filter(|line| line.starts_with("AUDIO")).collect::<Vec<_>>().join(",")
}

const DONE: &str = r#"{"version":1,"event":"task.done","title":"CC:A","announcement":"done"}"#;
const FAILED: &str = r#"{"version":1,"event":"task.error","title":"CC:B","announcement":"failed"}"#;
const INPUT: &str = r#"{"version":1,"event":"agent.input_required","title":"CC:C"}"#;

/// Three tasks finishing within seconds used to read out three lines back to back.
#[test]
fn a_burst_of_task_ends_speaks_once() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO QUEUED DONE");
    board.advance(2000);
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO MERGED DONE");
    board.advance(2000);
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO MERGED DONE");
    assert_eq!(board.played.len(), 1);

    // The window counts from the last line spoken, not the last end merged.
    board.advance(11_000);
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO QUEUED DONE");
}

#[test]
fn a_failure_after_a_done_still_speaks_but_not_the_other_way() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO QUEUED DONE");
    board.advance(1000);
    assert_eq!(last_audio(&mut firmware, &mut board, FAILED), "AUDIO QUEUED FAILED");
    board.advance(1000);
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO MERGED DONE");
    assert_eq!(last_audio(&mut firmware, &mut board, FAILED), "AUDIO MERGED FAILED");
}

#[test]
fn needs_input_always_speaks() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(last_audio(&mut firmware, &mut board, DONE), "AUDIO QUEUED DONE");
    board.advance(500);
    assert_eq!(last_audio(&mut firmware, &mut board, INPUT), "AUDIO QUEUED INPUT_REQUIRED");
}

#[test]
fn bad_lines_are_reported() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(send(&mut firmware, &mut board, "not json"), ["ERROR invalid_json"]);
    assert_eq!(send(&mut firmware, &mut board, r#"[1,2]"#), ["ERROR invalid_message"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":""}"#), ["ERROR invalid_message"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"x","title":3}"#), ["ERROR invalid_message"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":2,"event":"x"}"#), ["ERROR unsupported_version"]);
    let long = format!(r#"{{"version":1,"event":"x","title":"{}"}}"#, "a".repeat(4700));
    assert_eq!(send(&mut firmware, &mut board, &long), ["ERROR input_too_large"]);
    // A line ending in CRLF is accepted too.
    firmware.receive(&mut board, b"{\"version\":1,\"event\":\"device.identify\"}\r\n");
    assert_eq!(board.take_lines(), ["IDENTIFY"]);
}

#[test]
fn the_tally_and_volume_survive_a_reboot() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat","build":"def","hour":14,"day":20260926}"#);
    assert_eq!(lines, ["CLOCK HOUR 14", "POMODORO TODAY 0 0S DAY 20260926"]);
    // The same hour and the same day are not reported again.
    assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat","hour":14,"day":20260926}"#).is_empty());

    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.volume","level":80}"#), ["VOLUME 80"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.volume","level":5,"preview":true}"#), ["VOLUME 20", "AUDIO QUEUED DONE"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.volume","level":80}"#), ["VOLUME 80"]);

    // One focus session: press K0 to start, it ends 25 minutes later. A flip within 40 ms of boot counts as bounce.
    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(100);
    board.k0 = false;
    firmware.poll(&mut board);
    let lines = board.take_lines();
    assert!(lines.contains(&"POMODORO FOCUS START".to_owned()), "{lines:?}");
    assert!(lines.contains(&"MODE POMODORO".to_owned()), "{lines:?}");
    board.advance(25 * 60 * 1000);
    firmware.poll(&mut board);
    let lines = board.take_lines();
    assert!(lines.contains(&"POMODORO FOCUS END".to_owned()), "{lines:?}");
    assert!(lines.contains(&"POMODORO TODAY 1 1500S DAY 20260926".to_owned()), "{lines:?}");
    assert!(lines.contains(&"AUDIO QUEUED FOCUS_DONE".to_owned()), "{lines:?}");

    let flash = board.flash;
    let (_, board, lines) = booted(flash);
    assert_eq!(lines[0], "TALLY LOADED 1 1500S DAY 20260926");
    assert!(lines.contains(&"VOLUME 80".to_owned()));
    assert_eq!(board.codec_volume, Some(80));
}

#[test]
fn the_box_screens_switch_and_survive_a_reboot() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let at = |board: &FakeBoard, x: usize, y: usize| {
        let index = (y * 320 + x) * 2;
        u16::from_be_bytes([board.frame[index], board.frame[index + 1]])
    };
    // A spot left of Kore in korekore's side panel; the robot's screens have their background there.
    let robot = at(&board, 4, 150);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.screens","style":"korekore"}"#), ["SCREENS KOREKORE"]);
    // korekore's side panel is box-card.
    assert_eq!(at(&board, 4, 150), vibebuddy_firmware_core::korekore::theme::BOX_CARD);
    assert_ne!(at(&board, 4, 150), robot);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.screens","style":"retro"}"#), ["SCREENS ERROR", "SCREENS KOREKORE"]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.screens"}"#), ["SCREENS KOREKORE"]);

    let flash = board.flash;
    let (mut firmware, mut board, _) = booted(flash);
    assert_eq!(at(&board, 4, 150), vibebuddy_firmware_core::korekore::theme::BOX_CARD);
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
    assert!(lines.contains(&"SCREENS KOREKORE".to_owned()), "{lines:?}");
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.screens","style":"robot"}"#), ["SCREENS ROBOT"]);
    assert_eq!(at(&board, 4, 150), robot);
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..4 {
            out.push(if index <= chunk.len() { ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn voice_pack_bytes() -> Vec<u8> {
    let lengths = [4001u32, 4003, 4005, 4007, 4009];
    let payload: Vec<u8> = (0..lengths.iter().sum::<u32>()).map(|index| (index % 253) as u8).collect();
    let mut header = vec![0u8; 256];
    header[..4].copy_from_slice(b"VBVP");
    header[4..8].copy_from_slice(&1u32.to_le_bytes());
    header[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    header[12..16].copy_from_slice(&voice_pack::crc32(0, &payload).to_le_bytes());
    header[16..22].copy_from_slice(b"xiaohe");
    let mut offset = 256u32;
    for (index, length) in lengths.iter().enumerate() {
        header[48 + index * 4..52 + index * 4].copy_from_slice(&offset.to_le_bytes());
        header[68 + index * 4..72 + index * 4].copy_from_slice(&length.to_le_bytes());
        offset += length;
    }
    let crc = voice_pack::crc32(0, &header[..88]);
    header[88..92].copy_from_slice(&crc.to_le_bytes());
    header.extend_from_slice(&payload);
    header
}

#[test]
fn a_voice_pack_is_written_over_the_serial_line() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let pack = voice_pack_bytes();

    // Still making sound: stop playback first; don't erase the partition or reply ready.
    board.busy = true;
    let begin = format!(r#"{{"version":1,"event":"voice.begin","size":{}}}"#, pack.len());
    assert!(send(&mut firmware, &mut board, &begin).is_empty());
    assert_eq!(board.stops, 1);
    firmware.poll(&mut board);
    assert!(board.take_lines().is_empty());
    board.busy = false;
    firmware.poll(&mut board);
    assert_eq!(board.take_lines(), [r#"{"version":1,"event":"voice.ready","seq":-1}"#]);

    for (seq, piece) in pack.chunks(672).enumerate() {
        let chunk = format!(
            r#"{{"version":1,"event":"voice.chunk","seq":{seq},"data":"{}","crc":{}}}"#,
            encode_base64(piece),
            voice_pack::crc32(0, piece)
        );
        assert!(chunk.len() <= 1024, "a chunk must not exceed the protocol limit");
        assert_eq!(send(&mut firmware, &mut board, &chunk), [format!(r#"{{"version":1,"event":"voice.ack","seq":{seq}}}"#)]);
    }
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.end"}"#);
    assert_eq!(lines, [r#"{"version":1,"event":"voice.written","voice":"xiaohe"}"#, "VOICES xiaohe", "AUDIO QUEUED DONE"]);
    assert!(
        matches!(board.played.last(), Some(Sound::Line(line)) if line.codec == Codec::Pcm24kStereo),
        "says a line in the new voice when done"
    );

    // It is still used after a restart.
    let flash = board.flash;
    let (_, _, lines) = booted(flash);
    assert!(lines.contains(&"VOICES xiaohe".to_owned()));
}

#[test]
fn a_corrupted_chunk_is_rejected_with_its_sequence_number() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let pack = voice_pack_bytes();
    send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"voice.begin","size":{}}}"#, pack.len()));
    let chunk = format!(r#"{{"version":1,"event":"voice.chunk","seq":0,"data":"{}","crc":1}}"#, encode_base64(&pack[..672]));
    assert_eq!(
        send(&mut firmware, &mut board, &chunk),
        [r#"{"version":1,"event":"voice.error","seq":0,"message":"ESP_ERR_INVALID_CRC"}"#]
    );
    assert_eq!(
        send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.nope"}"#),
        [r#"{"version":1,"event":"voice.error","seq":-1,"message":"unknown voice event"}"#]
    );
}

#[test]
fn echo_reports_length_and_crc() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.echo","data":"123456789"}"#);
    assert_eq!(lines, [r#"{"version":1,"event":"echo","length":9,"crc":3421780262}"#, "ECHO 123456789"]);
}

#[test]
fn hello_repeats_the_static_state() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
    assert_eq!(lines, ["DISPLAY READY BUILD abc1234 2026-09-26 10:00", "FIRMWARE VERSION 1.2.3", "TRANSFER 1", "LINK NOISE", BOX_KEY, RELAY_URL, "MODE DUTY", "VOICES builtin", "VOLUME 65", "SCREENS ROBOT", "TOUCH NONE"]);
}

#[test]
fn silence_means_the_link_is_lost() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let presents = board.presents;
    board.advance(15_000);
    firmware.poll(&mut board);
    assert!(board.presents > presents);
}

#[test]
fn a_screenshot_is_run_length_encoded() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.screenshot"}"#);
    assert_eq!(lines.first().map(String::as_str), Some("SHOT BEGIN 320x240 BACKLIGHT ON"));
    assert_eq!(lines.last().map(String::as_str), Some("SHOT END"));
    let pixels: usize = lines[1..lines.len() - 1]
        .iter()
        .flat_map(|line| line.split(' ').skip(1))
        .map(|run| run.split(':').nth(1).unwrap().parse::<usize>().unwrap())
        .sum();
    assert_eq!(pixels, 320 * 240);
}

/// Polls every 10 ms until `SHOT END`, returning each `SHOT` line with the time it went out.
fn poll_screenshot(firmware: &mut Firmware, board: &mut FakeBoard) -> Vec<(u32, String)> {
    let mut shot = Vec::new();
    for _ in 0..20_000 {
        firmware.poll(board);
        let now = board.now_ms();
        shot.extend(board.take_lines().into_iter().filter(|line| line.starts_with("SHOT")).map(|line| (now, line)));
        if shot.last().is_some_and(|(_, line)| line == "SHOT END") {
            return shot;
        }
        board.advance(10);
    }
    panic!("the screenshot never finished: {} lines", shot.len());
}

/// The rgb565 pixels the `SHOT` lines between BEGIN and END describe.
fn shot_pixels(shot: &[(u32, String)]) -> Vec<u16> {
    let mut pixels = Vec::new();
    for (_, line) in &shot[1..shot.len() - 1] {
        for run in line.split(' ').skip(1) {
            let (color, count) = run.split_once(':').unwrap();
            let color = u16::from_str_radix(color, 16).unwrap();
            pixels.extend(std::iter::repeat_n(color, count.parse().unwrap()));
        }
    }
    pixels
}

#[test]
fn a_screenshot_over_the_relay_is_sent_a_line_at_a_time() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    firmware.relay_line(&mut board, br#"{"version":1,"event":"device.screenshot"}"#);
    assert!(!board.take_lines().iter().any(|line| line.starts_with("SHOT")), "nothing at once");
    let shot = poll_screenshot(&mut firmware, &mut board);
    assert_eq!(shot[0].1, "SHOT BEGIN 320x240 BACKLIGHT ON");
    assert_eq!(shot_pixels(&shot).len(), 320 * 240);
}

#[test]
fn a_screenshot_waits_while_the_network_cannot_take_a_whole_line() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    firmware.relay_line(&mut board, br#"{"version":1,"event":"device.screenshot"}"#);
    board.peer_full = true;
    for _ in 0..100 {
        firmware.poll(&mut board);
        board.advance(10);
    }
    assert!(!board.take_lines().iter().any(|line| line.starts_with("SHOT")), "nothing dropped half-way");
    board.peer_full = false;
    let shot = poll_screenshot(&mut firmware, &mut board);
    assert_eq!(shot[0].1, "SHOT BEGIN 320x240 BACKLIGHT ON");
    assert_eq!(shot_pixels(&shot).len(), 320 * 240);
}

#[test]
fn holding_k0_while_idle_does_nothing_and_a_tap_starts_focus() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(1100);
    firmware.poll(&mut board);
    board.advance(100);
    board.k0 = false;
    firmware.poll(&mut board);
    let lines = board.take_lines();
    assert!(!lines.iter().any(|line| line.starts_with("POMODORO")), "holding K0 while idle should do nothing: {lines:?}");

    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(100);
    board.k0 = false;
    firmware.poll(&mut board);
    assert_eq!(board.take_lines(), ["POMODORO FOCUS START", "MODE POMODORO"]);
}

#[test]
fn a_voice_pack_that_cannot_fit_is_refused_without_interrupting_playback() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.begin","size":99999999}"#);
    assert_eq!(lines, [r#"{"version":1,"event":"voice.error","seq":-1,"message":"ESP_ERR_INVALID_SIZE"}"#]);
    assert_eq!(board.stops, 0);
}

/// Writes a pack over the serial line the way the Mac does, with the box already quiet.
fn write_pack(firmware: &mut Firmware, board: &mut FakeBoard, pack: &[u8]) -> Vec<String> {
    let begin = format!(r#"{{"version":1,"event":"voice.begin","size":{}}}"#, pack.len());
    send(firmware, board, &begin);
    firmware.poll(board);
    board.take_lines();
    for (seq, piece) in pack.chunks(672).enumerate() {
        let chunk = format!(
            r#"{{"version":1,"event":"voice.chunk","seq":{seq},"data":"{}","crc":{}}}"#,
            encode_base64(piece),
            voice_pack::crc32(0, piece)
        );
        send(firmware, board, &chunk);
    }
    send(firmware, board, r#"{"version":1,"event":"voice.end"}"#)
}

/// Built by tools/character_pack.py: input required has two lines, done one, the evening greeting
/// one, and every other occasion none.
const SAMPLE_PACK: &[u8] = include_bytes!("../src/fixtures/sample_character_pack.bin");

fn adpcm_samples(sound: Option<&Sound>) -> Option<u32> {
    match sound {
        Some(Sound::Line(line)) => match line.codec {
            Codec::Adpcm16kMono { samples } => Some(samples),
            Codec::Pcm24kStereo => None,
        },
        _ => None,
    }
}

#[test]
fn a_character_pack_speaks_its_own_lines() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = write_pack(&mut firmware, &mut board, SAMPLE_PACK);
    assert_eq!(lines, [r#"{"version":1,"event":"voice.written","voice":"sample"}"#, "VOICES sample", "AUDIO QUEUED DONE"]);
    assert_eq!(adpcm_samples(board.played.last()), Some(3), "the done line of the new Character");

    // Its two needs-input lines take turns.
    let ask = r#"{"version":1,"event":"agent.input_required","title":"A"}"#;
    send(&mut firmware, &mut board, ask);
    let first = adpcm_samples(board.played.last());
    send(&mut firmware, &mut board, ask);
    let second = adpcm_samples(board.played.last());
    assert!(first.is_some() && second.is_some() && first != second, "{first:?} then {second:?}");

    // No failed pool: the built-in line.
    send(&mut firmware, &mut board, r#"{"version":1,"event":"task.error","title":"A"}"#);
    assert_eq!(board.played.last(), Some(&Sound::Builtin(Prompt::Failed)));
}

#[test]
fn a_special_occasion_picks_its_pool_or_falls_back() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    write_pack(&mut firmware, &mut board, SAMPLE_PACK);

    // The sample has no first-done pool, so the done line speaks for it.
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"task.done","title":"A","occasion":"first_done"}"#);
    assert_eq!(lines.last().map(String::as_str), Some("AUDIO QUEUED FIRST_DONE"));
    assert_eq!(adpcm_samples(board.played.last()), Some(3));

    // A special occasion that doesn't belong to the event is ignored.
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.input_required","title":"A","occasion":"first_done"}"#);
    assert_eq!(lines.last().map(String::as_str), Some("AUDIO QUEUED INPUT_REQUIRED"));
}

#[test]
fn the_daily_greeting_is_only_a_line() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    write_pack(&mut firmware, &mut board, SAMPLE_PACK);
    let played = board.played.len();

    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"buddy.say","occasion":"greeting_evening"}"#);
    assert_eq!(lines, ["AUDIO QUEUED GREETING_EVENING"]);
    assert_eq!(adpcm_samples(board.played.last()), Some(4));

    // No morning pool, and greetings have no built-in line: silence.
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"buddy.say","occasion":"greeting_morning"}"#);
    assert_eq!(lines, ["AUDIO SILENT GREETING_MORNING"]);

    // Only greetings may be said this way.
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"buddy.say","occasion":"done"}"#);
    assert!(lines.is_empty());
    assert_eq!(board.played.len(), played + 1);
}

#[derive(Clone, Copy)]
enum Key {
    K0,
    K1,
    K2,
}

/// Presses a key for `hold_ms` and lets go, polling along the way; returns what was printed.
fn press(firmware: &mut Firmware, board: &mut FakeBoard, key: Key, hold_ms: u32) -> Vec<String> {
    let set = |board: &mut FakeBoard, down: bool| match key {
        Key::K0 => board.k0 = down,
        Key::K1 => board.k1 = down,
        Key::K2 => board.k2 = down,
    };
    board.advance(100);
    set(board, true);
    firmware.poll(board);
    let mut held = 0;
    while held < hold_ms {
        board.advance(50);
        held += 50;
        firmware.poll(board);
    }
    set(board, false);
    firmware.poll(board);
    board.take_lines()
}

#[test]
fn k1_long_opens_the_menu_where_k0_steps_the_volume() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = press(&mut firmware, &mut board, Key::K1, 1100);
    assert_eq!(lines, ["MENU OPEN"], "a long K1 no longer starts leisure");
    // The first row is VOLUME: K0 moves 65 to the next step and plays the preview.
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["VOLUME 80", "AUDIO QUEUED DONE"]);
    assert_eq!(board.codec_volume, Some(80));
    // K1 moves to MUTE, K0 toggles it.
    assert!(press(&mut firmware, &mut board, Key::K1, 50).is_empty());
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["MUTE ON"]);
    // K2 closes rather than asking the Mac to open a source.
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 50), ["MENU CLOSED"]);
    // Closed, the keys are back to their own jobs.
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 50), [r#"{"version":1,"event":"button","button":"K2","action":"press"}"#]);
    assert_eq!(press(&mut firmware, &mut board, Key::K1, 50), ["MODE POMODORO"]);

    let flash = board.flash;
    let (_, board, lines) = booted(flash);
    assert!(lines.contains(&"VOLUME 80".to_owned()), "the menu's volume is saved: {lines:?}");
    assert_eq!(board.codec_volume, Some(80));
}

#[test]
fn stop_focus_is_offered_only_while_a_phase_runs() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["POMODORO FOCUS START", "MODE POMODORO"]);
    press(&mut firmware, &mut board, Key::K1, 1100);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["MENU CLOSED", "POMODORO STOPPED"]);
    // With nothing running the first row is VOLUME again.
    press(&mut firmware, &mut board, Key::K1, 1100);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["VOLUME 80", "AUDIO QUEUED DONE"]);
}

#[test]
fn an_agent_needing_the_user_closes_the_menu_and_quiet_closes_it_too() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    press(&mut firmware, &mut board, Key::K1, 1100);
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"task.start","title":"CC:A"}"#);
    assert!(!lines.contains(&"MENU CLOSED".to_owned()), "work in progress waits: {lines:?}");
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.input_required","title":"CC:A"}"#);
    assert!(lines.contains(&"MENU CLOSED".to_owned()), "{lines:?}");
    assert!(lines.contains(&"AUDIO QUEUED INPUT_REQUIRED".to_owned()), "{lines:?}");

    press(&mut firmware, &mut board, Key::K1, 1100);
    board.advance(29_000);
    firmware.poll(&mut board);
    assert!(!board.take_lines().contains(&"MENU CLOSED".to_owned()));
    board.advance(1_000);
    firmware.poll(&mut board);
    assert!(board.take_lines().contains(&"MENU CLOSED".to_owned()));
}

#[test]
fn k1_wraps_around_the_rows() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    // VOLUME, MUTE, COMPUTERS, NETWORK, STATUS, and back to VOLUME.
    press(&mut firmware, &mut board, Key::K1, 1100);
    for _ in 0..5 {
        press(&mut firmware, &mut board, Key::K1, 50);
    }
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["VOLUME 80", "AUDIO QUEUED DONE"]);
}

/// Built by tools/character_pack.py with a look from tools/make-look.py: four solid blocks in four colors.
const LOOK_PACK: &[u8] = include_bytes!("../src/fixtures/sample_look_pack.bin");

#[test]
fn a_pack_with_a_look_is_worn_and_survives_a_restart() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = write_pack(&mut firmware, &mut board, LOOK_PACK);
    assert!(lines.contains(&"LOOK looked".to_owned()), "{lines:?}");

    // The buddy is drawn from the look now: the robot's blue is gone from the screen.
    let robot_blue = [0x3c, 0x9f];
    assert!(!board.frame.chunks(2).any(|pixel| pixel == robot_blue), "no robot pixels left");

    let (mut firmware, mut board, lines) = booted(board.flash);
    assert!(lines.contains(&"LOOK looked".to_owned()), "after a restart: {lines:?}");

    // A pack without a look brings the robot back; the robot stands in from the moment the write
    // begins, which is when LOOK ROBOT is reported.
    write_pack(&mut firmware, &mut board, SAMPLE_PACK);
    assert!(board.frame.chunks(2).any(|pixel| pixel == robot_blue), "the robot is back");
}

const COMPUTER: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";

#[test]
fn the_box_keeps_its_key_and_pairings_across_restarts() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let pair = format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"dragon's MacBook"}}"#);
    assert_eq!(send(&mut firmware, &mut board, &pair), [format!("PAIRED {COMPUTER}")]);

    let (mut firmware, mut board, lines) = booted(board.flash);
    assert!(lines.contains(&BOX_KEY.to_owned()));
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pairs"}"#);
    assert_eq!(lines, ["PAIRS 1".to_owned(), format!("PAIR {COMPUTER} dragon's MacBook")]);
}

#[test]
fn unpairing_forgets_a_computer() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"x"}}"#));
    let lines = send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.unpair","key":"{COMPUTER}"}}"#));
    assert_eq!(lines, [format!("UNPAIRED {COMPUTER}")]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pairs"}"#), ["PAIRS 0"]);
}

#[test]
fn pairing_rejects_a_bad_key_and_isnt_agent_activity() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pair","key":"nope"}"#);
    assert_eq!(lines, ["PAIR INVALID"]);
}

#[test]
fn a_box_without_the_link_partition_has_no_key() {
    let mut flash = blank_flash();
    // Drop the link entry, as on the single-slot layout.
    flash.bytes[0x8000 + 6 * 32..0x8000 + 7 * 32].fill(0xFF);
    let (mut firmware, mut board, lines) = booted(flash);
    assert!(!lines.iter().any(|line| line.starts_with("BOX KEY")));
    let lines = send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}"}}"#));
    assert_eq!(lines, ["PAIR UNAVAILABLE"]);
}

#[test]
fn the_menu_forgets_every_computer_after_asking() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"x"}}"#));
    assert_eq!(press(&mut firmware, &mut board, Key::K1, 1100), ["MENU OPEN"]);
    // VOLUME, MUTE, then COMPUTERS, and FORGET ALL in it.
    press(&mut firmware, &mut board, Key::K1, 50);
    press(&mut firmware, &mut board, Key::K1, 50);
    assert!(press(&mut firmware, &mut board, Key::K0, 50).is_empty());
    assert!(press(&mut firmware, &mut board, Key::K0, 50).is_empty(), "the first K0 only asks");
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["UNPAIRED ALL"]);

    let (mut firmware, mut board, _) = booted(board.flash);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pairs"}"#), ["PAIRS 0"]);
}

#[test]
fn a_new_identity_forgets_everything_that_trusted_the_old_one() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"x"}}"#));
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.set","ssid":"Home","password":"secret"}"#);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.relay.set","url":"wss://relay-staging.korekore.ai"}"#);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.volume","level":35}"#);
    let open_reset = |firmware: &mut Firmware, board: &mut FakeBoard| {
        press(firmware, board, Key::K1, 1100);
        // VOLUME, MUTE, COMPUTERS; in it FORGET ALL, then NEW ID.
        press(firmware, board, Key::K1, 50);
        press(firmware, board, Key::K1, 50);
        press(firmware, board, Key::K0, 50);
        press(firmware, board, Key::K1, 50);
        press(firmware, board, Key::K0, 50);
    };

    // Backing out at the second ask changes nothing.
    open_reset(&mut firmware, &mut board);
    press(&mut firmware, &mut board, Key::K0, 50);
    press(&mut firmware, &mut board, Key::K1, 50);
    press(&mut firmware, &mut board, Key::K2, 50);
    assert_eq!(board.restarts, 0);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pairs"}"#)[0], "PAIRS 1");

    open_reset(&mut firmware, &mut board);
    press(&mut firmware, &mut board, Key::K0, 50);
    let lines = press(&mut firmware, &mut board, Key::K0, 50);
    let new_key = lines.iter().find_map(|line| line.strip_prefix("NEW IDENTITY ")).expect("the new key").to_owned();
    assert_ne!(format!("BOX KEY {new_key}"), BOX_KEY);
    assert_eq!(board.restarts, 1);

    let (mut firmware, mut board, lines) = booted(board.flash);
    assert!(lines.contains(&format!("BOX KEY {new_key}")), "{lines:?}");
    assert!(lines.contains(&"RELAY URL wss://relay.korekore.ai".to_owned()), "the Relay is the default again");
    assert!(lines.contains(&"VOLUME 35".to_owned()), "settings stay");
    assert!(!lines.iter().any(|line| line.starts_with("WIFI NETWORK")), "{lines:?}");
    assert_eq!(firmware.wifi(), None);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.pairs"}"#), ["PAIRS 0"]);
}

mod lan {
    use super::*;
    use ed25519_dalek::SigningKey;
    use serde_json::Value;
    use vibebuddy_firmware_core::lan::USB_QUIET_MS;
    use vibebuddy_firmware_core::link;
    use vibebuddy_firmware_core::noise::{Initiator, Sealer};

    fn computer() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    /// A box with the computer paired over the cable, the cable then gone quiet.
    fn paired_box() -> (Firmware, FakeBoard) {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let key = link::encode(&computer().verifying_key().to_bytes());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{key}","name":"a"}}"#));
        board.advance(USB_QUIET_MS);
        (firmware, board)
    }

    /// Opens connection `conn` and runs the Noise handshake; returns the computer's sealer.
    fn link_up(firmware: &mut Firmware, board: &mut FakeBoard, conn: u32) -> Sealer {
        firmware.lan_opened(board, conn, [5; 32]);
        let (_, opener) = board.lan_sent.pop().unwrap();
        let value: Value = serde_json::from_str(&opener).unwrap();
        let box_key = link::decode::<32>(value["box"].as_str().unwrap()).unwrap();
        let (initiator, first) = Initiator::start([2; 32], &box_key, [6; 32], b"{}").unwrap();
        firmware.lan_received(board, conn, format!("{first}\n").as_bytes());
        let (_, reply) = &board.lan_sent[board.lan_sent.len() - 2];
        let (sealer, mut from_box) = initiator.finish(reply.trim_end().as_bytes()).unwrap();
        let (_, welcome) = board.lan_sent.last().unwrap();
        assert_eq!(from_box.open(welcome.trim_end().as_bytes()).unwrap(), br#"{"version":1,"event":"link.welcome"}"#);
        board.take_lines();
        sealer
    }

    /// One protocol line from the linked computer, sealed.
    fn say(firmware: &mut Firmware, board: &mut FakeBoard, conn: u32, sealer: &mut Sealer, line: &str) {
        let sealed = sealer.seal(line.as_bytes()).unwrap();
        firmware.lan_received(board, conn, format!("{sealed}\n").as_bytes());
    }

    #[test]
    fn a_paired_computer_drives_the_box_over_the_network() {
        let (mut firmware, mut board) = paired_box();
        let mut sealer = link_up(&mut firmware, &mut board, 1);
        assert_eq!(board.lan_peer, Some(1), "the box's output goes to it");
        say(&mut firmware, &mut board, 1, &mut sealer, r#"{"version":1,"event":"device.hello"}"#);
        assert!(board.take_lines().contains(&"MODE DUTY".to_owned()));
    }

    #[test]
    fn a_screenshot_over_the_network_is_the_frame_when_asked_paced_for_the_relay() {
        let (mut firmware, mut board) = paired_box();
        let mut sealer = link_up(&mut firmware, &mut board, 1);
        // No two neighbouring pixels alike: the most runs a frame can have.
        let pattern: Vec<u16> = (0..320 * 240).map(|index| (index % 7) as u16 * 0x1111).collect();
        for (index, color) in pattern.iter().enumerate() {
            board.frame[index * 2..index * 2 + 2].copy_from_slice(&color.to_be_bytes());
        }
        say(&mut firmware, &mut board, 1, &mut sealer, r#"{"version":1,"event":"device.screenshot"}"#);
        assert!(!board.take_lines().iter().any(|line| line.starts_with("SHOT")), "nothing at once");
        // The screen moves on while the lines go out; the picture is the one asked for.
        board.frame.fill(0xff);
        let shot = poll_screenshot(&mut firmware, &mut board);
        assert_eq!(shot_pixels(&shot), pattern);
        // Every line is one Relay message: well under its 8 KiB, and the box's 4 KB network pipe.
        assert!(shot.iter().all(|(_, line)| line.len() < 2048), "longest {}", shot.iter().map(|(_, line)| line.len()).max().unwrap());
        // Under the Relay's 50 messages a second, burst or not.
        assert!(shot.windows(2).all(|pair| pair[1].0 - pair[0].0 >= 30));
    }

    #[test]
    fn pairing_and_wifi_are_cable_only() {
        let (mut firmware, mut board) = paired_box();
        let mut sealer = link_up(&mut firmware, &mut board, 1);
        for line in [
            r#"{"version":1,"event":"device.pair","key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#,
            r#"{"version":1,"event":"device.wifi.set","ssid":"evil","password":"x"}"#,
            r#"{"version":1,"event":"device.relay.set","url":"wss://evil.example"}"#,
        ] {
            say(&mut firmware, &mut board, 1, &mut sealer, line);
            assert_eq!(board.take_lines(), ["ERROR cable_only"]);
        }
    }

    #[test]
    fn the_cable_takes_the_box_back() {
        let (mut firmware, mut board) = paired_box();
        link_up(&mut firmware, &mut board, 1);
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat"}"#);
        assert!(board.lan_closed.contains(&1));
        assert_eq!(board.lan_peer_sent, [r#"{"version":1,"event":"link.unlinked","reason":"usb"}"#], "told, sealed");
        assert_eq!(board.lan_peer, None);
    }

    #[test]
    fn a_scan_is_asked_for_over_the_cable_only() {
        let (mut firmware, mut board) = paired_box();
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.scan"}"#).is_empty());
        assert!(firmware.take_scan_request());
        assert!(!firmware.take_scan_request(), "once");
        board.advance(USB_QUIET_MS);
        let mut sealer = link_up(&mut firmware, &mut board, 1);
        say(&mut firmware, &mut board, 1, &mut sealer, r#"{"version":1,"event":"device.wifi.scan"}"#);
        assert_eq!(board.take_lines(), ["ERROR cable_only"]);
        assert!(!firmware.take_scan_request());
    }

    #[test]
    fn the_uart_rate_is_asked_for_over_the_cable_only_and_quietly() {
        let (mut firmware, mut board) = paired_box();
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.baud","rate":921600}"#).is_empty());
        assert_eq!(board.uart_rates, [921_600]);
        // The check only has to arrive; it isn't agent activity, so no `EVENT` line.
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.baud.check"}"#).is_empty());
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.baud"}"#).is_empty(), "no rate, nothing to do");
        assert_eq!(board.uart_rates, [921_600]);
        board.advance(USB_QUIET_MS);
        let mut sealer = link_up(&mut firmware, &mut board, 1);
        say(&mut firmware, &mut board, 1, &mut sealer, r#"{"version":1,"event":"device.baud","rate":921600}"#);
        assert_eq!(board.take_lines(), ["ERROR cable_only"]);
        assert_eq!(board.uart_rates, [921_600]);
    }

    #[test]
    fn the_wifi_network_is_set_over_the_cable_and_kept() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.set","ssid":"Home","password":"secret"}"#);
        assert_eq!(lines, ["WIFI NETWORK Home"]);
        assert!(firmware.take_wifi_change());
        assert!(!firmware.take_wifi_change(), "only once");

        let (mut firmware, mut board, lines) = booted(board.flash);
        assert!(lines.contains(&"WIFI NETWORK Home".to_owned()), "{lines:?}");
        assert!(!lines.iter().any(|line| line.contains("secret")), "the password never leaves the box");
        assert_eq!(firmware.wifi().map(|wifi| wifi.password.as_str()), Some("secret"));
        assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.forget"}"#), ["WIFI FORGOTTEN"]);
        assert_eq!(firmware.wifi(), None);
    }

    #[test]
    fn hello_repeats_the_wifi_address_while_joined() {
        let (mut firmware, mut board) = paired_box();
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.set","ssid":"Home","password":"secret"}"#);
        let hello = |firmware: &mut Firmware, board: &mut _| send(firmware, board, r#"{"version":1,"event":"device.hello"}"#);
        assert!(!hello(&mut firmware, &mut board).iter().any(|line| line.starts_with("WIFI ADDRESS")), "not joined yet");
        firmware.set_wifi_joined(true);
        firmware.set_wifi_address("192.168.1.23");
        let lines = hello(&mut firmware, &mut board);
        let network = lines.iter().position(|line| line == "WIFI NETWORK Home").expect("network");
        assert_eq!(lines.get(network + 1).map(String::as_str), Some("WIFI ADDRESS 192.168.1.23"), "{lines:?}");
        firmware.set_wifi_joined(false);
        assert!(!hello(&mut firmware, &mut board).iter().any(|line| line.starts_with("WIFI ADDRESS")), "lost with the network");
    }

    #[test]
    fn the_screen_and_the_network_view_say_how_the_box_is_linked() {
        use vibebuddy_firmware_core::display::{LinkPath, Tone};
        use vibebuddy_firmware_core::menu::View;
        let values = |firmware: &Firmware, now| -> Vec<(String, Tone)> {
            firmware
                .menu_panel(View::Network, now)
                .lines
                .into_iter()
                .map(|(_, value, tone)| (String::from_utf8(value).unwrap(), tone))
                .collect()
        };
        let (mut firmware, mut board) = paired_box();
        let now = board.now_ms();
        assert_eq!(firmware.link_mark(now), (None, false), "no link shows nothing");
        assert_eq!(values(&firmware, now), [("NO LINK".to_owned(), Tone::Dim), ("OFF".to_owned(), Tone::Dim), ("OFF".to_owned(), Tone::Dim)]);

        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.set","ssid":"Home","password":"secret"}"#);
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.relay.set","url":"wss://relay-staging.korekore.ai"}"#);
        let now = board.now_ms();
        assert!(firmware.on_staging());
        assert_eq!(firmware.link_mark(now), (Some(LinkPath::Usb), false), "the cable doesn't go through the Relay");
        assert_eq!(values(&firmware, now)[1..], [("Home".to_owned(), Tone::Dim), ("STAGING DOWN".to_owned(), Tone::Dim)]);
        firmware.set_wifi_joined(true);
        firmware.set_relay_up(true);
        assert_eq!(values(&firmware, now), [("USB".to_owned(), Tone::Plain), ("Home".to_owned(), Tone::Plain), ("STAGING".to_owned(), Tone::Plain)]);

        board.advance(USB_QUIET_MS);
        link_up(&mut firmware, &mut board, 1);
        assert_eq!(firmware.link_mark(board.now_ms()), (Some(LinkPath::Lan), false));
        firmware.lan_closed(&mut board, 1);
        firmware.relay_linked(board.now_ms(), Some(computer().verifying_key().to_bytes()));
        assert_eq!(firmware.link_mark(board.now_ms()), (Some(LinkPath::Relay), true), "STG marks a link through staging");
    }

    #[test]
    fn the_relay_follows_the_computer_on_the_cable() {
        let (mut firmware, mut board, lines) = booted(blank_flash());
        assert!(lines.contains(&"RELAY URL wss://relay.korekore.ai".to_owned()), "{lines:?}");
        let staging = r#"{"version":1,"event":"device.relay.set","url":"wss://relay-staging.korekore.ai"}"#;
        assert_eq!(send(&mut firmware, &mut board, staging), ["RELAY URL wss://relay-staging.korekore.ai"]);
        assert!(firmware.take_relay_change());
        let saved = board.flash.bytes.clone();
        send(&mut firmware, &mut board, staging);
        assert!(!firmware.take_relay_change(), "the same Relay again changes nothing");
        assert!(board.flash.bytes == saved, "and writes nothing");
        assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.relay.set","url":"ws://plain"}"#), ["RELAY INVALID"]);

        let (mut firmware, mut board, lines) = booted(board.flash);
        assert!(lines.contains(&"RELAY URL wss://relay-staging.korekore.ai".to_owned()), "{lines:?}");
        assert_eq!(firmware.relay_url(), "wss://relay-staging.korekore.ai");
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.relay.set","url":"wss://relay.korekore.ai"}"#);
        assert!(firmware.take_relay_change());
        assert_eq!(firmware.relay_url(), "wss://relay.korekore.ai");
    }
}

mod relay {
    use super::*;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use serde_json::{Value, json};
    use vibebuddy_firmware_core::lan::USB_QUIET_MS;
    use vibebuddy_firmware_core::link;
    use vibebuddy_firmware_core::noise::{Initiator, Opener, Sealer};
    use vibebuddy_firmware_core::relay_box::{RelayBox, signed_message};

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// A computer whose secret the test knows, unlike [`COMPUTER`].
    fn computer_key() -> String {
        link::encode(&ed25519_dalek::SigningKey::from_bytes(&[2; 32]).verifying_key().to_bytes())
    }

    /// The Relay links the computer; it answers the box's `link.noise` and returns its session.
    fn session(relay: &mut RelayBox, firmware: &mut Firmware, board: &mut FakeBoard) -> (Sealer, Opener) {
        let opener = relay.message(firmware, board, &json!({"t": "linked", "computer": computer_key()}));
        let line = parse(&opener[0]);
        let noise = parse(line["line"].as_str().unwrap());
        assert_eq!(noise["event"], "link.noise");
        let box_key = link::decode_key(noise["box"].as_str().unwrap()).unwrap();
        let (initiator, first) = Initiator::start([2; 32], &box_key, [6; 32], b"{}").unwrap();
        let reply = relay.message(firmware, board, &json!({"t": "data", "line": first}));
        initiator.finish(parse(&reply[0])["line"].as_str().unwrap().as_bytes()).unwrap()
    }

    fn say(relay: &mut RelayBox, firmware: &mut Firmware, board: &mut FakeBoard, sealer: &mut Sealer, line: &str) {
        relay.message(firmware, board, &json!({"t": "data", "line": sealer.seal(line.as_bytes()).unwrap()}));
    }

    #[test]
    fn the_box_signs_reports_and_carries_lines() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"a"}}"#));
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{}","name":"b"}}"#, computer_key()));
        let mut relay = RelayBox::new();

        let hello = parse(&relay.hello(&firmware, "abc").unwrap());
        let box_id = hello["key"].as_str().unwrap().to_owned();
        let key = VerifyingKey::from_bytes(&link::decode_key(&box_id).unwrap()).unwrap();
        let sig = Signature::from_bytes(&link::decode::<64>(hello["sig"].as_str().unwrap()).unwrap());
        assert!(key.verify(&signed_message("box", &box_id, "abc"), &sig).is_ok());
        assert!(relay.report(&firmware, board.now_ms()).is_empty(), "nothing before welcome");

        relay.message(&mut firmware, &mut board, &json!({"t": "welcome"}));
        let reports: Vec<Value> = relay.report(&firmware, board.now_ms()).iter().map(|text| parse(text)).collect();
        assert_eq!(reports[0], json!({"t": "paired", "computers": [{"key": COMPUTER, "name": "a"}, {"key": computer_key(), "name": "b"}]}));
        assert_eq!(reports[1], json!({"t": "linked", "via": "usb", "computer": computer_key()}), "it was just paired over the cable");
        assert!(relay.report(&firmware, board.now_ms()).is_empty(), "only changes");

        board.advance(USB_QUIET_MS);
        assert_eq!(relay.report(&firmware, board.now_ms()), [r#"{"t":"unlinked"}"#]);

        let (mut sealer, _) = session(&mut relay, &mut firmware, &mut board);
        assert!(relay.has_peer());
        assert!(board.relay_peer, "the box's output goes to it, sealed");
        board.take_lines();
        say(&mut relay, &mut firmware, &mut board, &mut sealer, r#"{"version":1,"event":"device.hello"}"#);
        assert!(board.take_lines().contains(&"MODE DUTY".to_owned()));

        // A line in the clear is never acted on; one that doesn't open starts the session over.
        relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": r#"{"version":1,"event":"device.hello"}"#}));
        assert!(board.take_lines().is_empty());
        assert!(!relay.has_peer());

        relay.message(&mut firmware, &mut board, &json!({"t": "unlinked", "reason": "takeover"}));
        assert!(!relay.has_peer());
        assert!(!board.relay_peer);
    }

    #[test]
    fn every_new_socket_hears_the_local_link_again() {
        // The Relay forgets a local owner when the box's socket goes, so a box that reconnects while still on
        // the cable must claim it again.
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"a"}}"#));
        let mut relay = RelayBox::new();
        for socket in ["first", "second"] {
            relay.hello(&firmware, socket);
            relay.message(&mut firmware, &mut board, &json!({"t": "welcome"}));
            let reports: Vec<Value> = relay.report(&firmware, board.now_ms()).iter().map(|text| parse(text)).collect();
            assert_eq!(reports[1], json!({"t": "linked", "via": "usb", "computer": COMPUTER}), "{socket}");
            relay.gone(&mut firmware, &mut board);
        }
    }

    #[test]
    fn unlinked_ends_only_the_computers_link() {
        // `unlinked` ends the link through the Relay, not the socket: the box keeps reporting and the Relay stays up.
        use vibebuddy_firmware_core::display::Tone;
        use vibebuddy_firmware_core::menu::View;
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"a"}}"#));
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.set","ssid":"Home","password":"secret"}"#);
        let mut relay = RelayBox::new();
        relay.hello(&firmware, "n");
        relay.message(&mut firmware, &mut board, &json!({"t": "welcome"}));
        relay.report(&firmware, board.now_ms());
        board.advance(USB_QUIET_MS);
        relay.report(&firmware, board.now_ms());
        relay.message(&mut firmware, &mut board, &json!({"t": "linked", "computer": COMPUTER}));

        relay.message(&mut firmware, &mut board, &json!({"t": "unlinked", "reason": "takeover"}));
        assert!(!relay.has_peer());
        let relay_line = |firmware: &Firmware| firmware.menu_panel(View::Network, board.now_ms()).lines[2].clone();
        assert_eq!(relay_line(&firmware), (b"RELAY".to_vec(), b"PROD".to_vec(), Tone::Plain), "the socket is still up");

        let other = link::encode(&[3; 32]);
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{other}","name":"b"}}"#));
        let reports: Vec<Value> = relay.report(&firmware, board.now_ms()).iter().map(|text| parse(text)).collect();
        assert_eq!(reports.len(), 2, "{reports:?}");
        assert_eq!(reports[0]["t"], "paired");
        assert_eq!(reports[1], json!({"t": "linked", "via": "usb", "computer": other}));
    }

    #[test]
    fn pairing_over_the_relay_is_refused() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{}","name":"b"}}"#, computer_key()));
        board.advance(USB_QUIET_MS);
        let mut relay = RelayBox::new();
        relay.hello(&firmware, "n");
        let (mut sealer, _) = session(&mut relay, &mut firmware, &mut board);
        board.take_lines();
        say(&mut relay, &mut firmware, &mut board, &mut sealer, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}"}}"#));
        assert_eq!(board.take_lines(), ["ERROR cable_only"]);
    }

    #[test]
    fn only_the_computer_the_relay_linked_gets_a_session() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{}","name":"b"}}"#, computer_key()));
        let mut relay = RelayBox::new();
        relay.message(&mut firmware, &mut board, &json!({"t": "linked", "computer": COMPUTER}));
        let box_key = firmware.link().unwrap().public_key();
        let (_, first) = Initiator::start([2; 32], &box_key, [6; 32], b"{}").unwrap();
        assert!(relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": first})).is_empty());
        assert!(!relay.has_peer());
    }

    #[test]
    fn a_box_that_missed_linked_takes_a_handshake_from_a_paired_computer() {
        // The computer linked while the box was restarting, and the box never heard `linked`: the computer starts
        // the handshake itself (daemon/src/relay.rs).
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{}","name":"b"}}"#, computer_key()));
        board.advance(USB_QUIET_MS);
        let mut relay = RelayBox::new();
        relay.message(&mut firmware, &mut board, &json!({"t": "welcome"}));
        let box_key = firmware.link().unwrap().public_key();
        let (initiator, first) = Initiator::start([2; 32], &box_key, [6; 32], b"{}").unwrap();
        let reply = relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": first}));
        let (mut sealer, _) = initiator.finish(parse(&reply[0])["line"].as_str().unwrap().as_bytes()).unwrap();
        assert!(relay.has_peer() && board.relay_peer);
        assert_eq!(firmware.link_mark(board.now_ms()).0, Some(vibebuddy_firmware_core::display::LinkPath::Relay), "linked through the Relay now");
        board.take_lines();
        say(&mut relay, &mut firmware, &mut board, &mut sealer, r#"{"version":1,"event":"device.hello"}"#);
        assert!(board.take_lines().contains(&"MODE DUTY".to_owned()));

        // A second handshake while the session is up doesn't open as a line: the box starts over and says so.
        let (_, again) = Initiator::start([2; 32], &box_key, [7; 32], b"{}").unwrap();
        let answer = relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": again}));
        assert_eq!(parse(parse(&answer[0])["line"].as_str().unwrap())["event"], "link.noise");
        assert!(!relay.has_peer());
    }
}

mod transfer {
    use super::*;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use vibebuddy_firmware_core::transfer::CHUNK_BYTES;
    use vibebuddy_firmware_core::voice_pack::crc32;

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let bits = chunk.iter().enumerate().fold(0u32, |acc, (index, &byte)| acc | u32::from(byte) << (16 - 8 * index));
            for index in 0..4 {
                out.push(if index <= chunk.len() { ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    fn reply(lines: &[String]) -> Value {
        serde_json::from_str(lines.last().unwrap()).unwrap()
    }

    /// Sends xfer.begin and runs the main loop until the box answers: it erases the slot first.
    fn begin_transfer(firmware: &mut Firmware, board: &mut FakeBoard, line: &str) -> Value {
        let mut lines = send(firmware, board, line);
        let mut passes = 0;
        while !lines.iter().any(|line| line.contains("xfer.")) {
            firmware.poll(board);
            lines = board.take_lines();
            passes += 1;
            assert!(passes < 100, "no answer to xfer.begin");
        }
        reply(&lines)
    }

    #[test]
    fn a_firmware_image_comes_over_the_link_and_the_box_restarts_into_it() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let mut image: Vec<u8> = (0..10_000).map(|index| (index * 13 + 1) as u8).collect();
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        let sha: String = Sha256::digest(&image).iter().map(|byte| format!("{byte:02x}")).collect();
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":{},"sha256":"{sha}"}}"#, image.len());
        assert_eq!(begin_transfer(&mut firmware, &mut board, &begin), serde_json::json!({"version": 1, "event": "xfer.ready", "offset": 0}));
        for (index, piece) in image.chunks(CHUNK_BYTES).enumerate() {
            let line = format!(
                r#"{{"version":1,"event":"xfer.chunk","offset":{},"crc":{},"data":"{}"}}"#,
                index * CHUNK_BYTES,
                crc32(0, piece),
                base64(piece)
            );
            assert!(line.len() < 4608 || piece.len() < CHUNK_BYTES, "a full chunk fits a line");
            let ack = reply(&send(&mut firmware, &mut board, &line));
            assert_eq!(ack["offset"], (index * CHUNK_BYTES + piece.len()) as u64);
        }
        // Asked again, the same image resumes at its end.
        assert_eq!(reply(&send(&mut firmware, &mut board, &begin))["offset"], image.len() as u64);
        assert_eq!(reply(&send(&mut firmware, &mut board, r#"{"version":1,"event":"xfer.end"}"#))["event"], "xfer.done");
        assert_eq!(board.restarts, 1);
        assert_eq!(&board.flash.bytes[0x610000..0x610000 + image.len()], &image[..]);
    }

    #[test]
    fn a_chunk_of_a_full_size_survives_the_line_limit() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let mut image = vec![0xA5u8; CHUNK_BYTES * 2];
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        let sha: String = Sha256::digest(&image).iter().map(|byte| format!("{byte:02x}")).collect();
        begin_transfer(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":{},"sha256":"{sha}"}}"#, image.len()));
        let piece = &image[..CHUNK_BYTES];
        let line = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, piece), base64(piece));
        assert_eq!(reply(&send(&mut firmware, &mut board, &line))["offset"], CHUNK_BYTES as u64);
    }

    /// Over the UART bridge the main loop holds bytes until their line is whole. It once kept only 2048 of them and
    /// lost the newline, so every chunk ran into the next and the box answered `input_too_large` (2026-10-26).
    #[test]
    fn a_full_chunk_over_the_uart_is_held_whole() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let mut image = vec![0xA5u8; CHUNK_BYTES * 2];
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        let sha: String = Sha256::digest(&image).iter().map(|byte| format!("{byte:02x}")).collect();
        begin_transfer(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":{},"sha256":"{sha}"}}"#, image.len()));
        let uart = |firmware: &mut Firmware, board: &mut FakeBoard, bytes: &[u8]| {
            let mut held = Vec::new();
            for &byte in bytes {
                if hold_line_byte(&mut held, byte) {
                    firmware.receive(board, &held);
                    held.clear();
                }
            }
            board.take_lines()
        };
        let too_long = format!("{}\n", "x".repeat(5000));
        assert_eq!(uart(&mut firmware, &mut board, too_long.as_bytes()), ["ERROR input_too_large"]);
        let mut lines = Vec::new();
        for (index, piece) in image.chunks(CHUNK_BYTES).enumerate() {
            lines.extend_from_slice(
                format!(r#"{{"version":1,"event":"xfer.chunk","offset":{},"crc":{},"data":"{}"}}"#, index * CHUNK_BYTES, crc32(0, piece), base64(piece)).as_bytes(),
            );
            lines.push(b'\n');
        }
        let acks: Vec<Value> = uart(&mut firmware, &mut board, &lines).iter().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(acks.iter().map(|ack| ack["offset"].as_u64().unwrap()).collect::<Vec<_>>(), [CHUNK_BYTES as u64, 2 * CHUNK_BYTES as u64]);
    }

    /// Sends a whole image of `kind` over xfer.*; returns what the box said to xfer.end.
    fn send_over_xfer(firmware: &mut Firmware, board: &mut FakeBoard, kind: &str, image: &[u8], sha: &str) -> Vec<String> {
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"{kind}","size":{},"sha256":"{sha}"}}"#, image.len());
        assert_eq!(begin_transfer(firmware, board, &begin)["event"], "xfer.ready");
        for (index, piece) in image.chunks(CHUNK_BYTES).enumerate() {
            let line = format!(r#"{{"version":1,"event":"xfer.chunk","offset":{},"crc":{},"data":"{}"}}"#, index * CHUNK_BYTES, crc32(0, piece), base64(piece));
            assert_eq!(reply(&send(firmware, board, &line))["event"], "xfer.ack");
        }
        send(firmware, board, r#"{"version":1,"event":"xfer.end"}"#)
    }

    fn hex_sha(image: &[u8]) -> String {
        Sha256::digest(image).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn a_character_pack_comes_over_the_link_and_speaks_at_once() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let lines = send_over_xfer(&mut firmware, &mut board, "character", SAMPLE_PACK, &hex_sha(SAMPLE_PACK));
        assert_eq!(lines, [r#"{"version":1,"event":"xfer.done","voice":"sample"}"#, "VOICES sample", "AUDIO QUEUED DONE"]);
        assert_eq!(adpcm_samples(board.played.last()), Some(3), "the done line of the new Character");
        assert_eq!(board.restarts, 0, "a pack needs no restart");
    }

    #[test]
    fn while_a_pack_comes_in_the_built_in_voice_speaks_and_a_failed_one_leaves_it() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        send_over_xfer(&mut firmware, &mut board, "character", SAMPLE_PACK, &hex_sha(SAMPLE_PACK));
        // Another pack, with a hash that won't match: set aside while it comes, and gone after.
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"character","size":{},"sha256":"{}"}}"#, SAMPLE_PACK.len(), "00".repeat(32));
        begin_transfer(&mut firmware, &mut board, &begin);
        send(&mut firmware, &mut board, r#"{"version":1,"event":"task.done","title":"A"}"#);
        assert_eq!(board.played.last(), Some(&Sound::Builtin(Prompt::Done)), "the partition is being rewritten");
        let line = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, SAMPLE_PACK), base64(SAMPLE_PACK));
        send(&mut firmware, &mut board, &line);
        assert_eq!(reply(&send(&mut firmware, &mut board, r#"{"version":1,"event":"xfer.end"}"#))["code"], "sha256");
        let hello = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
        assert!(hello.contains(&"VOICES builtin".to_owned()), "the old pack was erased, nothing new took its place: {hello:?}");
    }

    #[test]
    fn something_else_is_not_a_pack() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let junk = vec![0x42u8; 2000];
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"character","size":{},"sha256":"{}"}}"#, junk.len(), hex_sha(&junk));
        begin_transfer(&mut firmware, &mut board, &begin);
        let line = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, &junk), base64(&junk));
        assert_eq!(reply(&send(&mut firmware, &mut board, &line))["code"], "bad_image");
    }

    #[test]
    fn the_screen_shows_the_update_and_only_a_long_k2_counts() {
        use vibebuddy_firmware_core::display::PanelKind;
        let mut board = FakeBoard::new(blank_flash());
        board.touchscreen = true;
        let mut firmware = Firmware::new(1, board.now_ms(), b"abc1234 2026-09-26 10:00", "1.2.3");
        firmware.boot(&mut board);
        board.take_lines();
        let image = vec![0xA5u8; CHUNK_BYTES * 4];
        let mut image = image;
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":{},"sha256":"{}"}}"#, image.len(), hex_sha(&image));
        send(&mut firmware, &mut board, &begin);
        let panel = firmware.overlay().expect("a panel while erasing");
        assert_eq!((panel.title.as_slice(), panel.kind), (&b"UPDATING"[..], PanelKind::Progress(0)));
        assert_eq!(panel.lines[0].0, b"GETTING READY");
        begin_transfer(&mut firmware, &mut board, &begin);
        let piece = &image[..CHUNK_BYTES];
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, piece), base64(piece)));
        let panel = firmware.overlay().unwrap();
        assert_eq!((panel.kind, panel.lines[0].0.as_slice()), (PanelKind::Progress(25), &b"25%"[..]));
        // The panel draws a hint's first 12 characters (seen cut on a box as "HOLD TO CANC").
        assert!(panel.hints.iter().all(|(_, action)| action.len() <= 12));

        // Announcements wait for the restart; K0 and K1 and a short K2 do nothing; a long K2 cancels.
        let played = board.played.len();
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"task.done","title":"A"}"#).contains(&"AUDIO UPDATING DONE".to_owned()));
        assert_eq!(board.played.len(), played, "silent");
        for key in [Key::K0, Key::K1, Key::K2] {
            assert!(!press(&mut firmware, &mut board, key, 100).iter().any(|line| line.contains("MODE") || line.contains("button")));
        }
        assert!(press(&mut firmware, &mut board, Key::K1, 1500).iter().all(|line| !line.contains("MENU OPEN")), "not even the menu");
        // Nor does a tap.
        board.finger = Some((5, 5));
        for _ in 0..4 {
            firmware.poll(&mut board);
            board.advance(20);
        }
        board.finger = None;
        firmware.poll(&mut board);
        assert!(firmware.overlay().is_some_and(|panel| panel.title == b"UPDATING"));
        let lines = press(&mut firmware, &mut board, Key::K2, 1500);
        assert!(lines.contains(&r#"{"version":1,"event":"xfer.error","code":"cancelled"}"#.to_owned()), "{lines:?}");
        assert!(firmware.overlay().is_none(), "the screen comes back");
        assert!(press(&mut firmware, &mut board, Key::K1, 100).iter().any(|line| line.starts_with("MODE")), "and the keys");
    }

    #[test]
    fn a_pack_shows_as_a_new_character() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"character","size":{},"sha256":"{}"}}"#, SAMPLE_PACK.len(), hex_sha(SAMPLE_PACK));
        begin_transfer(&mut firmware, &mut board, &begin);
        assert_eq!(firmware.overlay().unwrap().title, b"NEW CHARACTER");
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, SAMPLE_PACK), base64(SAMPLE_PACK)));
        send(&mut firmware, &mut board, r#"{"version":1,"event":"xfer.end"}"#);
        assert!(firmware.overlay().is_none());
    }

    #[test]
    fn a_transfer_its_computer_left_is_given_up() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"character","size":{},"sha256":"{}"}}"#, SAMPLE_PACK.len(), hex_sha(SAMPLE_PACK));
        begin_transfer(&mut firmware, &mut board, &begin);
        board.advance(4 * 60_000);
        firmware.poll(&mut board);
        assert!(firmware.overlay().is_some(), "four minutes is still a link coming back");
        board.advance(60_000);
        firmware.poll(&mut board);
        assert!(board.take_lines().contains(&"XFER ABANDONED".to_owned()));
        assert!(firmware.overlay().is_none());
    }

    #[test]
    fn a_bad_chunk_ends_the_transfer_with_its_reason() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":100,"sha256":"{}"}}"#, "00".repeat(32));
        begin_transfer(&mut firmware, &mut board, &begin);
        let line = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":{},"data":"{}"}}"#, crc32(0, &[0x55; 100]), base64(&[0x55; 100]));
        assert_eq!(reply(&send(&mut firmware, &mut board, &line)), serde_json::json!({"version": 1, "event": "xfer.error", "code": "bad_image"}));
        assert_eq!(reply(&send(&mut firmware, &mut board, r#"{"version":1,"event":"xfer.end"}"#))["code"], "sequence");
        assert_eq!(board.restarts, 0);
    }

    /// The UART bridge changes bytes on the way: a chunk that fails its CRC, or whose base64 doesn't decode, is asked
    /// for again with the offset the box still expects, and the transfer carries on.
    #[test]
    fn a_damaged_chunk_is_asked_for_again() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let mut image = vec![0xA5u8; 100];
        image[0] = 0xE9;
        image[12..14].copy_from_slice(&[9, 0]);
        let begin = format!(r#"{{"version":1,"event":"xfer.begin","kind":"firmware","size":100,"sha256":"{}"}}"#, "00".repeat(32));
        begin_transfer(&mut firmware, &mut board, &begin);
        let ack = |offset: u64| serde_json::json!({"version": 1, "event": "xfer.ack", "offset": offset});
        let bad_crc = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":1,"data":"{}"}}"#, base64(&image));
        assert_eq!(reply(&send(&mut firmware, &mut board, &bad_crc)), ack(0));
        let bad_base64 = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"crc":1,"data":"{}"}}"#, "!".repeat(136));
        assert_eq!(reply(&send(&mut firmware, &mut board, &bad_base64)), ack(0));
        let good = format!(r#"{{"version":1,"event":"xfer.chunk","offset":0,"round":7,"crc":{},"data":"{}"}}"#, crc32(0, &image), base64(&image));
        assert_eq!(reply(&send(&mut firmware, &mut board, &good)), serde_json::json!({"version": 1, "event": "xfer.ack", "offset": 100, "round": 7}), "the round comes back");
    }
}
/// Polls until `MIC END` and returns every line.
fn poll_mic(firmware: &mut Firmware, board: &mut FakeBoard) -> Vec<String> {
    let mut lines = Vec::new();
    for _ in 0..2_000 {
        board.advance(10);
        firmware.poll(board);
        lines.extend(board.take_lines());
        if lines.last().is_some_and(|line| line.starts_with("MIC END")) {
            return lines;
        }
    }
    panic!("no MIC END; got {} lines", lines.len());
}

#[test]
fn a_mic_recording_reports_probes_levels_and_the_samples() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record","ms":200}"#);
    assert_eq!(
        lines,
        [
            "MIC PROBE 0x18 ID 8311",
            "MIC PROBE 0x40 NONE",
            "MIC PROBE 0x41 NONE",
            "MIC PROBE 0x42 NONE",
            "MIC PROBE 0x43 NONE",
            "MIC GAIN 30 DB",
            "MIC RECORDING 200 MS",
        ]
    );
    // es8311_set_mic_gain(30 dB) writes 5 to register 0x16.
    assert_eq!(board.i2c_writes, [(0x18, 0x16, 5)]);
    assert_eq!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record"}"#), ["MIC BUSY"]);

    let lines = poll_mic(&mut firmware, &mut board);
    assert_eq!(lines[0], "MIC CAPTURED 10 MS 19200 BYTES");
    assert_eq!(&lines[1..3], ["MIC LEVEL 0 L 144 255 R 0 0", "MIC LEVEL 100 L 147 255 R 0 0"]);
    assert_eq!(lines[3], "MIC BEGIN 24000 2 16 19200");
    let data: Vec<&String> = lines.iter().filter(|line| line.starts_with("MIC DATA ")).collect();
    assert_eq!(data.len(), 19200usize.div_ceil(576));
    let mut samples = Vec::new();
    for (seq, line) in data.iter().enumerate() {
        assert!(line.len() < 800, "a data line stays well under the 1 KB line limit");
        let mut parts = line.splitn(4, ' ').skip(2);
        assert_eq!(parts.next().unwrap(), seq.to_string());
        let mut out = vec![0u8; 576];
        let length = voice_pack::decode_base64(parts.next().unwrap().as_bytes(), &mut out).unwrap();
        samples.extend_from_slice(&out[..length]);
    }
    let expected: Vec<u8> = (0..4800).flat_map(|frame| [(frame % 256) as u8, 0, 0, 0]).collect();
    assert_eq!(samples, expected);
    assert_eq!(lines.last().unwrap(), &format!("MIC END 19200 {}", voice_pack::crc32(0, &expected)));

    // Done: another recording may start.
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record","ms":100}"#);
    assert_eq!(lines.last().map(String::as_str), Some("MIC RECORDING 100 MS"));
}

#[test]
fn a_mic_recording_is_clamped_and_reports_what_is_missing() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    board.i2c.clear();
    board.i2c.insert((0x41, 0x3D), 0x72);
    board.i2c.insert((0x41, 0x3E), 0x10);
    board.mic_input = false;
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record","ms":60000,"gain_db":99}"#);
    assert_eq!(
        lines,
        [
            "MIC PROBE 0x18 NONE",
            "MIC PROBE 0x40 NONE",
            "MIC PROBE 0x41 ID 7210",
            "MIC PROBE 0x42 NONE",
            "MIC PROBE 0x43 NONE",
            "MIC UNAVAILABLE",
        ]
    );
    // No ES8311: no gain write.
    assert!(board.i2c_writes.is_empty());

    board.mic_input = true;
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record","ms":60000}"#);
    assert_eq!(lines.last().map(String::as_str), Some("MIC RECORDING 5000 MS"));
    assert_eq!(board.mic_capture, Some(480_000));
}

const VOICE_ON: &str = r#"{"version":1,"event":"voice.available","on":true}"#;

/// Holds K0 for `hold_ms`, with the microphone delivering 24 samples a millisecond while it runs; returns
/// the lines printed (the `VOICE DATA` lines are in `board.stream`).
fn hold_k0_talking(firmware: &mut Firmware, board: &mut FakeBoard, hold_ms: u32) -> Vec<String> {
    board.advance(100);
    board.k0 = true;
    firmware.poll(board);
    let mut held = 0;
    while held < hold_ms {
        board.advance(10);
        held += 10;
        if board.voice_running {
            board.voice_samples.extend((0..240).map(|index| if index % 2 == 0 { 4000 } else { -4000 }));
        }
        firmware.poll(board);
    }
    board.k0 = false;
    firmware.poll(board);
    board.take_lines()
}

#[test]
fn without_voice_a_long_k0_still_gives_up_the_phase() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["POMODORO FOCUS START", "MODE POMODORO"]);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 1100), ["POMODORO STOPPED"]);
    assert!(!board.voice_running);
    // Offered and then withdrawn, it's the same.
    send(&mut firmware, &mut board, VOICE_ON);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.available","on":false}"#);
    press(&mut firmware, &mut board, Key::K0, 50);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 1100), ["POMODORO STOPPED"]);
}

#[test]
fn with_voice_off_for_a_reason_a_long_k0_says_why_while_pomodoro_is_idle() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.available","on":false,"reason":"allowance","message":"Today's voice is used up. It comes back at 08:00."}"#);
    assert!(firmware.overlay().is_none(), "nothing shows until K0");
    let lines = hold_k0_talking(&mut firmware, &mut board, 1500);
    assert!(lines.is_empty(), "nothing recorded, no phase given up: {lines:?}");
    assert!(board.stream.is_empty() && !board.voice_running);
    let panel = firmware.overlay().expect("the reason");
    assert_eq!(panel.title, b"VOICE USED UP");
    let text: Vec<String> = panel.lines.iter().map(|(row, _, _)| String::from_utf8(row.clone()).unwrap()).collect();
    assert_eq!(text.join(" "), "Today's voice is used up. It comes back at 08:00.");
    // It goes after 6 s.
    board.advance(6_000);
    firmware.poll(&mut board);
    assert!(firmware.overlay().is_none());

    // While Pomodoro runs, a long K0 still gives up the phase.
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["POMODORO FOCUS START", "MODE POMODORO"]);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 1100), ["POMODORO STOPPED"]);
    assert!(firmware.overlay().is_none());
}

#[test]
fn a_refusal_after_speaking_shows_its_title_and_the_computers_message() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    hold_k0_talking(&mut firmware, &mut board, 1800);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.error","turn":1,"reason":"global_cap","message":"Voice is paused for everyone today. It comes back at 08:00."}"#);
    let panel = firmware.overlay().expect("the error");
    assert_eq!(panel.title, b"VOICE PAUSED");
    let text: Vec<String> = panel.lines.iter().map(|(row, _, _)| String::from_utf8(row.clone()).unwrap()).collect();
    assert_eq!(text.join(" "), "Voice is paused for everyone today. It comes back at 08:00.");
}

#[test]
fn with_voice_off_and_no_reason_a_long_k0_shows_nothing() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.available","on":false}"#);
    assert!(hold_k0_talking(&mut firmware, &mut board, 1500).is_empty());
    assert!(firmware.overlay().is_none());
    // Nor after a new link, which forgets the last reason.
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.available","on":false,"reason":"signed_out"}"#);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
    board.take_lines();
    assert!(hold_k0_talking(&mut firmware, &mut board, 1500).is_empty());
    assert!(firmware.overlay().is_none());
}

#[test]
fn with_voice_a_long_k0_records_a_turn_and_the_phase_keeps_running() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["POMODORO FOCUS START", "MODE POMODORO"]);
    assert!(send(&mut firmware, &mut board, VOICE_ON).is_empty());
    let lines = hold_k0_talking(&mut firmware, &mut board, 2000);
    // Recording starts when the long press fires, a second in, and runs until K0 comes up.
    assert_eq!(lines.first().map(String::as_str), Some("VOICE START 1"));
    assert!(!lines.iter().any(|line| line.starts_with("POMODORO")), "{lines:?}");
    let end = lines.iter().find(|line| line.starts_with("VOICE END 1 ")).expect("VOICE END");
    let samples: usize = end.rsplit(' ').next().unwrap().parse().unwrap();
    assert!((22_000..=24_500).contains(&samples), "about a second at 24 kHz: {samples}");
    assert!(!board.voice_running, "the microphone stops with the recording");
    // Every data line went to the stream, numbered without gaps, the last one shorter.
    assert_eq!(board.stream.len(), samples.div_ceil(1152));
    for (seq, line) in board.stream.iter().enumerate() {
        assert!(line.starts_with(&format!("VOICE DATA 1 {seq} ")) && line.ends_with('\n') && line.len() < 1024, "{line}");
    }
    // The microphone ran at voice gain: 42 dB is register 0x16 = 7.
    assert!(board.i2c_writes.contains(&(0x18, 0x16, 7)));
    // The overlay counts as the user being there.
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 50), Vec::<String>::new(), "K2 closes the overlay");
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["POMODORO PAUSED"], "the phase was never given up");
}

#[test]
fn a_tap_past_the_long_press_is_cancelled() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    let lines = hold_k0_talking(&mut firmware, &mut board, 1300);
    assert_eq!(lines, ["VOICE START 1", "VOICE CANCEL 1"]);
    assert!(!board.voice_running);
}

#[test]
fn the_computers_answers_fill_the_overlay_and_k2_closes_it() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    hold_k0_talking(&mut firmware, &mut board, 1800);
    let before = board.presents;
    assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.heard","turn":1,"text":"what is codex doing"}"#).is_empty());
    assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.reply","turn":1,"text":"Writing tests."}"#).is_empty());
    assert!(board.presents >= before + 2, "each answer redraws the overlay");
    // Another turn's answers change nothing.
    let before = board.presents;
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.reply","turn":9,"text":"nope"}"#);
    assert_eq!(board.presents, before);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.done","turn":1}"#);
    // K2 closes it rather than asking the computer to open a source; the next K2 does that again.
    assert!(press(&mut firmware, &mut board, Key::K2, 50).is_empty());
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 50), [r#"{"version":1,"event":"button","button":"K2","action":"press"}"#]);
}

#[test]
fn a_needs_input_event_waits_for_a_voice_command_and_is_said_after_it() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(1100);
    firmware.poll(&mut board);
    assert_eq!(board.take_lines(), ["VOICE START 1"]);
    board.played.clear();
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.input_required","title":"CC:ASK"}"#);
    assert!(!lines.iter().any(|line| line.starts_with("VOICE CANCEL")), "the recording goes on: {lines:?}");
    assert!(lines.contains(&"AUDIO HELD INPUT_REQUIRED".to_owned()), "{lines:?}");
    assert!(board.voice_running);
    assert!(board.played.is_empty(), "nothing is said over the conversation");
    // The same Agent asking again is still one thing held.
    send(&mut firmware, &mut board, r#"{"version":1,"event":"agent.input_required","title":"CC:ASK"}"#);
    board.advance(500);
    board.k0 = false;
    firmware.poll(&mut board);
    assert!(board.take_lines().iter().any(|line| line.starts_with("VOICE END 1")));
    assert!(board.played.is_empty());
    // K2 closes the overlay, and the held news is said once.
    let lines = press(&mut firmware, &mut board, Key::K2, 50);
    assert!(lines.contains(&"AUDIO QUEUED INPUT_REQUIRED".to_owned()), "{lines:?}");
    assert_eq!(board.played.len(), 1, "{:?}", board.played);
}

#[test]
fn a_new_link_starts_with_voice_off() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
    press(&mut firmware, &mut board, Key::K0, 50);
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 1100), ["POMODORO STOPPED"]);
    // A link that goes quiet takes voice with it too.
    send(&mut firmware, &mut board, VOICE_ON);
    board.advance(16_000);
    firmware.poll(&mut board);
    board.take_lines();
    assert!(hold_k0_talking(&mut firmware, &mut board, 1500).iter().all(|line| !line.starts_with("VOICE")));
}

#[test]
fn a_box_that_cannot_record_says_so_and_sends_nothing() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    board.mic_input = false;
    send(&mut firmware, &mut board, VOICE_ON);
    let before = board.presents;
    let lines = hold_k0_talking(&mut firmware, &mut board, 1500);
    assert!(lines.iter().all(|line| !line.starts_with("VOICE") && !line.starts_with("POMODORO")), "{lines:?}");
    assert!(board.presents > before, "the overlay says there's no microphone");
}

#[test]
fn the_diagnostic_and_a_voice_command_share_the_microphone() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    send(&mut firmware, &mut board, VOICE_ON);
    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(1100);
    firmware.poll(&mut board);
    board.take_lines();
    let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.mic.record"}"#);
    assert_eq!(lines, ["MIC BUSY"]);
}

const REPLY_MP3: &[u8] = include_bytes!("../src/fixtures/spoken_reply.mp3");
const K2_PRESS: &str = r#"{"version":1,"event":"button","button":"K2","action":"press"}"#;

/// A turn recorded and answered with a reply's text: the box is waiting on turn 1's audio.
fn answered_turn(firmware: &mut Firmware, board: &mut FakeBoard) {
    send(firmware, board, VOICE_ON);
    hold_k0_talking(firmware, board, 1800);
    send(firmware, board, r#"{"version":1,"event":"voice.reply","turn":1,"text":"Writing tests."}"#);
    board.reply_audio.clear();
}

/// A reply's MP3 as the computer sends it: 576 bytes (768 base64 characters) a line, then the end.
fn audio_lines(turn: u32, reply: u32, mp3: &[u8], end: &str) -> Vec<String> {
    let mut lines: Vec<String> = mp3
        .chunks(576)
        .enumerate()
        .map(|(seq, piece)| format!(r#"{{"version":1,"event":"voice.audio","turn":{turn},"reply":{reply},"seq":{seq},"data":"{}"}}"#, encode_base64(piece)))
        .collect();
    lines.push(format!(r#"{{"version":1,"event":"voice.audio","turn":{turn},"reply":{reply},"seq":{},"end":true{end}}}"#, mp3.len().div_ceil(576)));
    lines
}

/// What a player makes of what the board was handed.
fn pcm_of(audio: &[ReplyAudio]) -> Vec<u8> {
    let mut player = ReplyPlayer::new();
    for audio in audio {
        player.apply(audio.clone());
    }
    let mut out = Vec::new();
    let mut buffer = [0u8; 1000];
    loop {
        let count = player.fill(&mut buffer);
        if count == 0 {
            return out;
        }
        out.extend_from_slice(&buffer[..count]);
    }
}

#[test]
fn a_spoken_replys_audio_reaches_the_player_whole() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    for line in audio_lines(1, 0, REPLY_MP3, "") {
        assert!(line.len() < 1024);
        assert!(send(&mut firmware, &mut board, &line).is_empty(), "{line}");
    }
    let chunks = REPLY_MP3.len().div_ceil(576);
    assert_eq!(board.reply_audio.len(), chunks + 1);
    assert_eq!(board.reply_audio.last(), Some(&ReplyAudio::End { reply: 0, seq: chunks as u32 }));
    let received: Vec<u8> = board
        .reply_audio
        .iter()
        .flat_map(|audio| match audio {
            ReplyAudio::Chunk { data, .. } => data.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert!(received == REPLY_MP3, "the bytes arrive intact");
    assert!(pcm_of(&board.reply_audio).len() > 24_000 * 4 * 3, "seconds of speech");
    // Another turn's audio, or audio without a turn, goes nowhere.
    board.reply_audio.clear();
    send(&mut firmware, &mut board, &audio_lines(7, 0, &REPLY_MP3[..576], "")[0]);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.audio","reply":0,"seq":0,"data":"AAAA"}"#);
    assert!(board.reply_audio.is_empty());
}

#[test]
fn a_reply_cut_short_by_synthesis_plays_what_came_and_says_so() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    let mut printed = Vec::new();
    for line in audio_lines(1, 0, &REPLY_MP3[..576 * 5], r#","error":"synthesis""#) {
        printed.extend(send(&mut firmware, &mut board, &line));
    }
    assert_eq!(printed, ["VOICE AUDIO CUT SHORT 1 0"]);
    assert_eq!(board.reply_audio.last(), Some(&ReplyAudio::End { reply: 0, seq: 5 }));
    assert!(!pcm_of(&board.reply_audio).is_empty(), "what came still plays");
    // The text stays up to be read.
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.done","turn":1}"#);
    assert!(press(&mut firmware, &mut board, Key::K2, 50).is_empty(), "K2 closes the overlay");
}

#[test]
fn while_muted_replies_are_shown_and_not_played() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    let lines = audio_lines(1, 0, &REPLY_MP3[..576 * 4], "");
    send(&mut firmware, &mut board, &lines[0]);
    assert_eq!(board.reply_audio.len(), 1);
    // Muting mid-reply (K2 long): the reply stops at once, and the rest of the turn stays silent.
    board.reply_audio.clear();
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 1100), ["MUTE ON"]);
    assert_eq!(board.reply_audio, [ReplyAudio::Stop]);
    for line in &lines[1..] {
        send(&mut firmware, &mut board, line);
    }
    assert_eq!(board.reply_audio, [ReplyAudio::Stop], "nothing more reaches the player");

    // Muted before the turn: nothing reaches the player at all, and the text still shows.
    let (mut firmware, mut board, _) = booted(blank_flash());
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 1100), ["MUTE ON"]);
    answered_turn(&mut firmware, &mut board);
    for line in audio_lines(1, 0, &REPLY_MP3[..576 * 4], r#","error":"synthesis""#) {
        send(&mut firmware, &mut board, &line);
    }
    assert!(board.reply_audio.is_empty());
    assert!(press(&mut firmware, &mut board, Key::K2, 50).is_empty(), "the overlay is up");
}

#[test]
fn an_agents_news_during_a_reply_waits_and_the_reply_plays_on() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    let lines = audio_lines(1, 0, &REPLY_MP3[..576 * 6], "");
    for line in &lines[..3] {
        send(&mut firmware, &mut board, line);
    }
    board.reply_audio.clear();
    board.played.clear();
    let out = send(
        &mut firmware,
        &mut board,
        r#"{"version":1,"event":"task.done","title":"CC:VIBE","tasks":[{"title":"CC:VIBE","status":"done","elapsed_s":3}]}"#,
    );
    assert!(out.contains(&"AUDIO HELD DONE".to_owned()), "{out:?}");
    assert!(!board.reply_audio.contains(&ReplyAudio::Stop), "the reply isn't cut off");
    assert!(board.played.is_empty(), "nothing is said over the reply");
    for line in &lines[3..] {
        send(&mut firmware, &mut board, line);
    }
    assert!(board.reply_audio.iter().any(|audio| matches!(audio, ReplyAudio::Chunk { .. })), "the reply's audio still plays");
    // Once the overlay closes, the news is said.
    press(&mut firmware, &mut board, Key::K2, 50);
    assert_eq!(board.played, [Sound::Builtin(Prompt::Done)]);
}

#[test]
fn the_pomodoro_chime_stops_a_reply_too() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    press(&mut firmware, &mut board, Key::K0, 50);
    answered_turn(&mut firmware, &mut board);
    send(&mut firmware, &mut board, &audio_lines(1, 0, &REPLY_MP3[..576], "")[0]);
    board.reply_audio.clear();
    board.advance(25 * 60 * 1000);
    firmware.poll(&mut board);
    assert!(board.take_lines().contains(&"POMODORO FOCUS END".to_owned()));
    assert_eq!(board.reply_audio, [ReplyAudio::Stop]);
}

#[test]
fn a_new_push_to_talk_stops_the_reply_and_plays_the_new_turns() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    send(&mut firmware, &mut board, &audio_lines(1, 0, &REPLY_MP3[..576], "")[0]);
    board.reply_audio.clear();
    board.advance(100);
    board.k0 = true;
    firmware.poll(&mut board);
    board.advance(1100);
    firmware.poll(&mut board);
    assert!(board.voice_running);
    assert_eq!(board.reply_audio, [ReplyAudio::Stop], "the reply stops before the box listens");
    board.advance(1000);
    board.k0 = false;
    firmware.poll(&mut board);
    board.take_lines();
    send(&mut firmware, &mut board, &audio_lines(1, 1, &REPLY_MP3[..576], "")[0]);
    assert_eq!(board.reply_audio, [ReplyAudio::Stop], "the old turn's audio is ignored");
    send(&mut firmware, &mut board, &audio_lines(2, 0, &REPLY_MP3[..576], "")[0]);
    assert_eq!(board.reply_audio.len(), 2, "the new turn's plays");
}

#[test]
fn the_text_stays_up_while_a_reply_is_spoken_and_k2_stops_it() {
    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    send(&mut firmware, &mut board, &audio_lines(1, 0, &REPLY_MP3[..576], "")[0]);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"voice.done","turn":1}"#);
    board.speaking = true;
    // The link stays up past the 15 s read time.
    board.advance(10_000);
    send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat"}"#);
    board.advance(6_000);
    firmware.poll(&mut board);
    assert_eq!(board.reply_audio.last(), Some(&ReplyAudio::Chunk { reply: 0, seq: 0, data: REPLY_MP3[..576].to_vec() }), "past the read time, still speaking");
    board.speaking = false;
    board.advance(10);
    firmware.poll(&mut board);
    assert_eq!(board.reply_audio.last(), Some(&ReplyAudio::Stop), "the overlay closes, and what was left to say goes with it");
    assert_eq!(press(&mut firmware, &mut board, Key::K2, 50), [K2_PRESS]);

    let (mut firmware, mut board, _) = booted(blank_flash());
    answered_turn(&mut firmware, &mut board);
    send(&mut firmware, &mut board, &audio_lines(1, 0, &REPLY_MP3[..576], "")[0]);
    board.reply_audio.clear();
    assert!(press(&mut firmware, &mut board, Key::K2, 50).is_empty());
    assert_eq!(board.reply_audio, [ReplyAudio::Stop]);
}

mod touchscreen {
    use super::*;

    fn touch_box() -> (Firmware, FakeBoard, Vec<String>) {
        let mut board = FakeBoard::new(blank_flash());
        board.touchscreen = true;
        let mut firmware = Firmware::new(7, board.now_ms(), b"abc1234 2026-09-26 10:00", "1.2.3");
        firmware.boot(&mut board);
        let lines = board.take_lines();
        (firmware, board, lines)
    }

    /// A finger down at (x, y) for about 80 ms, polling along the way; returns what was printed.
    fn tap(firmware: &mut Firmware, board: &mut FakeBoard, x: i32, y: i32) -> Vec<String> {
        board.advance(100);
        board.finger = Some((x, y));
        for _ in 0..4 {
            firmware.poll(board);
            board.advance(20);
        }
        board.finger = None;
        firmware.poll(board);
        board.take_lines()
    }

    fn pixel(board: &FakeBoard, x: usize, y: usize) -> u16 {
        let at = (y * 320 + x) * 2;
        u16::from_be_bytes([board.frame[at], board.frame[at + 1]])
    }

    /// The middle of the menu list's row `index`.
    fn row(index: i32) -> (i32, i32) {
        (160, 61 + index * 23)
    }

    #[test]
    fn a_touchscreen_is_reported_at_boot_and_on_hello() {
        let (mut firmware, mut board, lines) = touch_box();
        assert!(lines.contains(&"TOUCH CHSC5432".to_owned()), "{lines:?}");
        let lines = send(&mut firmware, &mut board, r#"{"version":1,"event":"device.hello"}"#);
        assert_eq!(lines.last().map(String::as_str), Some("TOUCH CHSC5432"));
    }

    #[test]
    fn a_box_without_one_never_reads_it() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        board.finger = Some((160, 100));
        assert!(tap(&mut firmware, &mut board, 160, 100).is_empty());
    }

    #[test]
    fn a_tapped_menu_row_is_selected_and_acted_on() {
        let (mut firmware, mut board, _) = touch_box();
        press(&mut firmware, &mut board, Key::K1, 1100);
        // VOLUME, MUTE, COMPUTERS, NETWORK, STATUS.
        let (x, y) = row(1);
        assert_eq!(tap(&mut firmware, &mut board, x, y), [format!("TAP {x} {y}"), "MUTE ON".to_owned()]);
        let (x, y) = row(0);
        assert_eq!(tap(&mut firmware, &mut board, x, y), [format!("TAP {x} {y}"), "VOLUME 80".to_owned(), "AUDIO MUTED DONE".to_owned()]);
        // The keys still work alongside, from the row the finger left selected.
        assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["VOLUME 100", "AUDIO MUTED DONE"]);
    }

    /// The panel's inside, which covers whatever the screen underneath is doing.
    fn panel(board: &FakeBoard) -> Vec<u16> {
        (20..196).flat_map(|y| (34..286).map(move |x| (x, y))).map(|(x, y)| pixel(board, x, y)).collect()
    }

    #[test]
    fn under_the_last_row_is_still_the_last_row() {
        let (mut firmware, mut board, _) = touch_box();
        // STATUS by the keys: VOLUME, MUTE, COMPUTERS, NETWORK, STATUS.
        press(&mut firmware, &mut board, Key::K1, 1100);
        for _ in 0..4 {
            press(&mut firmware, &mut board, Key::K1, 50);
        }
        press(&mut firmware, &mut board, Key::K0, 50);
        let status = panel(&board);
        press(&mut firmware, &mut board, Key::K2, 50);

        // On the box a finger aimed at STATUS landed 15 px under it, between the row and the hints.
        press(&mut firmware, &mut board, Key::K1, 1100);
        tap(&mut firmware, &mut board, 160, 185);
        assert!(panel(&board) == status, "the tap opened STATUS");
    }

    #[test]
    fn a_tapped_key_hint_is_that_key() {
        let (mut firmware, mut board, _) = touch_box();
        press(&mut firmware, &mut board, Key::K1, 1100);
        // K1 NEXT, K0 CHANGE, K2 CLOSE along the bottom.
        assert_eq!(tap(&mut firmware, &mut board, 60, 210), ["TAP 60 210"]);
        assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["MUTE ON"], "K1's hint moved to MUTE");
        assert_eq!(tap(&mut firmware, &mut board, 190, 210), ["TAP 190 210", "MENU CLOSED"]);
        // On the STATUS view the hints are BACK and CLOSE, and the empty bar to the right is CLOSE too.
        press(&mut firmware, &mut board, Key::K1, 1100);
        tap(&mut firmware, &mut board, 160, 185);
        assert_eq!(tap(&mut firmware, &mut board, 260, 210), ["TAP 260 210", "MENU CLOSED"]);
        // Away from the panel, on the dimmed edge, is K2 too.
        press(&mut firmware, &mut board, Key::K1, 1100);
        assert_eq!(tap(&mut firmware, &mut board, 305, 130), ["TAP 305 130", "MENU CLOSED"]);
        press(&mut firmware, &mut board, Key::K1, 1100);
        assert_eq!(tap(&mut firmware, &mut board, 5, 130), ["TAP 5 130", "MENU CLOSED"]);
        // Closed, the same place is just the screen.
        assert_eq!(tap(&mut firmware, &mut board, 190, 210), ["TAP 190 210"]);
    }

    #[test]
    fn petting_the_buddy_changes_nothing_but_the_screen() {
        let (mut firmware, mut board, _) = touch_box();
        assert_ne!(pixel(&board, 202, 34), 0xF800);
        assert_eq!(tap(&mut firmware, &mut board, 160, 100), ["TAP 160 100"]);
        assert_eq!(pixel(&board, 202, 34), 0xF800, "a heart over the buddy");
        board.advance(1500);
        firmware.poll(&mut board);
        assert_ne!(pixel(&board, 202, 34), 0xF800, "and it goes");
        // Off the buddy there's nothing to tap.
        tap(&mut firmware, &mut board, 20, 200);
        assert_ne!(pixel(&board, 202, 34), 0xF800);
    }

    #[test]
    fn a_tapped_card_asks_for_its_agent() {
        let (mut firmware, mut board, _) = touch_box();
        send(
            &mut firmware,
            &mut board,
            r#"{"version":1,"event":"task.start","title":"B","tasks":[{"title":"B","status":"working","elapsed_s":5,"id":"9f2c01ab"},{"title":"A","status":"input_required","elapsed_s":60,"id":"0d4e77c3"}]}"#,
        );
        // The second card sits 38 px under the first.
        assert_eq!(tap(&mut firmware, &mut board, 100, 104), ["TAP 100 104", r#"{"version":1,"event":"focus","task":"0d4e77c3"}"#]);
        assert_eq!(tap(&mut firmware, &mut board, 100, 66), ["TAP 100 66", r#"{"version":1,"event":"focus","task":"9f2c01ab"}"#]);
        // Under the cards there's nothing; in Pomodoro there are no cards.
        assert_eq!(tap(&mut firmware, &mut board, 100, 130), ["TAP 100 130"]);
        press(&mut firmware, &mut board, Key::K1, 50);
        assert_eq!(tap(&mut firmware, &mut board, 100, 66), ["TAP 100 66"]);
    }

    #[test]
    fn a_card_from_an_older_daemon_or_with_an_odd_id_names_nothing() {
        let (mut firmware, mut board, _) = touch_box();
        send(
            &mut firmware,
            &mut board,
            r#"{"version":1,"event":"task.start","title":"B","tasks":[{"title":"B","status":"working","elapsed_s":5},{"title":"A","status":"working","elapsed_s":6,"id":"a\"b"}]}"#,
        );
        assert_eq!(tap(&mut firmware, &mut board, 100, 66), ["TAP 100 66"]);
        assert_eq!(tap(&mut firmware, &mut board, 100, 104), ["TAP 100 104"]);
    }

    #[test]
    fn a_slide_is_not_a_tap() {
        let (mut firmware, mut board, _) = touch_box();
        press(&mut firmware, &mut board, Key::K1, 1100);
        board.advance(100);
        board.finger = Some((160, 61));
        firmware.poll(&mut board);
        board.advance(40);
        board.finger = Some((160, 130));
        firmware.poll(&mut board);
        board.advance(40);
        board.finger = None;
        firmware.poll(&mut board);
        assert_eq!(board.take_lines(), ["SWIPE DOWN"], "a swipe, and in the menu it turns nothing");
    }

    /// A finger from (x, y) to (x, to_y) in 120 ms.
    fn swipe(firmware: &mut Firmware, board: &mut FakeBoard, x: i32, y: i32, to_y: i32) -> Vec<String> {
        board.advance(100);
        for step in 0..=6 {
            board.finger = Some((x, y + (to_y - y) * step / 6));
            firmware.poll(board);
            board.advance(20);
        }
        board.finger = None;
        firmware.poll(board);
        board.take_lines()
    }

    /// Five working cards, ids c1 to c5, the first from Claude Code, and seven activities in all.
    const FIVE_CARDS: &str = r#"{"version":1,"event":"task.start","title":"CC:ONE","tasks_total":7,"tasks":[
        {"id":"c1","agent":"claude-code","title":"CC:ONE","status":"working","elapsed_s":1},
        {"id":"c2","agent":"codex","title":"CX:TWO","status":"working","elapsed_s":2},
        {"id":"c3","agent":"github-actions","title":"CI:THREE","status":"working","elapsed_s":3},
        {"id":"c4","agent":"opencode","title":"OC:FOUR","status":"working","elapsed_s":4},
        {"id":"c5","agent":"unknown","title":"XX:FIVE","status":"working","elapsed_s":5}]}"#;

    fn focus(id: &str) -> String {
        format!(r#"{{"version":1,"event":"focus","task":"{id}"}}"#)
    }

    #[test]
    fn a_swipe_turns_the_cards_page_and_taps_follow_it() {
        let (mut firmware, mut board, _) = touch_box();
        send(&mut firmware, &mut board, &FIVE_CARDS.replace('\n', ""));
        assert_eq!(pixel(&board, 22, 58), 0xDBAA, "Claude's spark in its orange, in place of CC:");
        assert_eq!(tap(&mut firmware, &mut board, 100, 66)[1], focus("c1"));

        assert_eq!(swipe(&mut firmware, &mut board, 100, 150, 60), ["SWIPE UP", "CARDS PAGE 2/2"]);
        assert_eq!(tap(&mut firmware, &mut board, 100, 66)[1], focus("c4"));
        assert_eq!(tap(&mut firmware, &mut board, 100, 104), ["TAP 100 104", &focus("c5")]);
        assert_eq!(tap(&mut firmware, &mut board, 100, 142), ["TAP 100 142"], "page 2 has two cards");

        // The page line under the cards turns it too, and down goes back.
        assert_eq!(tap(&mut firmware, &mut board, 160, 212), ["TAP 160 212", "CARDS PAGE 1/2"]);
        assert_eq!(swipe(&mut firmware, &mut board, 100, 60, 150), ["SWIPE DOWN", "CARDS PAGE 2/2"]);
        // News starts from the first page.
        send(&mut firmware, &mut board, &FIVE_CARDS.replace('\n', ""));
        assert_eq!(tap(&mut firmware, &mut board, 100, 66)[1], focus("c1"));
    }

    #[test]
    fn the_pages_turn_by_themselves_unless_a_card_waits_or_a_finger_just_turned_them() {
        let (mut firmware, mut board, _) = touch_box();
        send(&mut firmware, &mut board, &FIVE_CARDS.replace('\n', ""));
        let top = |board: &FakeBoard| pixel(board, 22, 58);
        assert_eq!(top(&board), 0xDBAA);
        board.advance(8_100);
        firmware.poll(&mut board);
        assert_eq!(top(&board), 0xFFFF, "page 2 opens with OpenCode's white frame");
        board.advance(8_100);
        firmware.poll(&mut board);
        assert_eq!(top(&board), 0xDBAA, "and wraps round");

        // A finger's turn holds the page for 30 s.
        swipe(&mut firmware, &mut board, 100, 150, 60);
        board.advance(20_000);
        firmware.poll(&mut board);
        assert_eq!(top(&board), 0xFFFF);
        board.advance(11_000);
        firmware.poll(&mut board);
        assert_eq!(top(&board), 0xDBAA);

        // A card waiting for an answer keeps the first page up.
        let waiting = FIVE_CARDS.replace('\n', "").replacen(r#""status":"working""#, r#""status":"input_required""#, 1);
        send(&mut firmware, &mut board, &waiting.replace("task.start", "agent.input_required"));
        board.advance(20_000);
        firmware.poll(&mut board);
        assert_eq!(top(&board), 0xDBAA);
    }

    #[test]
    fn a_tap_calls_the_buddy_back_from_leisure_and_does_nothing_else() {
        let (mut firmware, mut board, _) = touch_box();
        let mut lines = Vec::new();
        for _ in 0..(10 * 60 / 5) {
            board.advance(5_000);
            lines.extend(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat"}"#));
            firmware.poll(&mut board);
            lines.extend(board.take_lines());
        }
        assert!(lines.contains(&"MODE LEISURE".to_owned()), "{lines:?}");
        // Where the buddy stands on duty: in Leisure it's no pet, only a way back.
        assert_eq!(tap(&mut firmware, &mut board, 160, 100), ["TAP 160 100", "MODE DUTY", "LEISURE ALERT"]);
        assert_ne!(pixel(&board, 202, 34), 0xF800);
    }
}
