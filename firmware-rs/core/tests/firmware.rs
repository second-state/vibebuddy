//! Host tests for the whole chain: a fake board, JSON fed over serial, then check what came
//! back, what was stored and what was played. The expected output lines are taken from the C
//! firmware's behavior, and the Mac's parsing relies on them staying verbatim.

use std::cell::Cell;

use vibebuddy_firmware_core::audio::{Codec, Prompt, Sound};
use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::storage::{Flash, FlashError};
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
    /// The linked network computer's buffer is full.
    peer_full: bool,
    /// Restarts asked for.
    restarts: usize,
    /// Registers of the chips on I2C, by (address, register), and what was written.
    i2c: std::collections::BTreeMap<(u8, u8), u8>,
    i2c_writes: Vec<(u8, u8, u8)>,
    /// The board has I2S input; and the size of the capture asked for, until it is taken.
    mic_input: bool,
    mic_capture: Option<usize>,
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
            peer_full: false,
            restarts: 0,
            i2c: [((0x18, 0xFD), 0x83), ((0x18, 0xFE), 0x11), ((0x18, 0x16), 0x24)].into(),
            i2c_writes: Vec::new(),
            mic_input: true,
            mic_capture: None,
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
    fn lan_peer(&mut self, conn: Option<u32>) {
        self.lan_peer = conn;
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
            BOX_KEY,
            RELAY_URL,
            "VOICES builtin",
            "AUDIO READY",
            "AUDIO CODEC ES8311",
            "VOLUME 65",
            "BUTTONS READY",
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
    let long = format!(r#"{{"version":1,"event":"x","title":"{}"}}"#, "a".repeat(1100));
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
    assert_eq!(lines, ["DISPLAY READY BUILD abc1234 2026-09-26 10:00", "FIRMWARE VERSION 1.2.3", BOX_KEY, RELAY_URL, "MODE DUTY", "VOICES builtin", "VOLUME 65"]);
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
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::Value;
    use vibebuddy_firmware_core::lan::{USB_QUIET_MS, signed_message};
    use vibebuddy_firmware_core::link;

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

    /// Opens connection `conn` and answers the challenge.
    fn link_up(firmware: &mut Firmware, board: &mut FakeBoard, conn: u32) {
        firmware.lan_opened(board, conn, [5; 32]);
        let (_, challenge) = board.lan_sent.pop().unwrap();
        let value: Value = serde_json::from_str(&challenge).unwrap();
        let box_key = link::decode::<32>(value["box"].as_str().unwrap()).unwrap();
        let nonce = link::decode::<32>(value["nonce"].as_str().unwrap()).unwrap();
        let sig = computer().sign(&signed_message("computer", &box_key, &nonce)).to_bytes();
        let hello = format!(
            "{{\"version\":1,\"event\":\"link.hello\",\"key\":\"{}\",\"sig\":\"{}\",\"nonce\":\"{}\"}}\n",
            link::encode(&computer().verifying_key().to_bytes()),
            link::encode(&sig),
            link::encode(&[6; 32])
        );
        firmware.lan_received(board, conn, hello.as_bytes());
        assert!(board.lan_sent.last().unwrap().1.contains("link.welcome"), "{:?}", board.lan_sent);
        board.take_lines();
    }

    #[test]
    fn a_paired_computer_drives_the_box_over_the_network() {
        let (mut firmware, mut board) = paired_box();
        link_up(&mut firmware, &mut board, 1);
        assert_eq!(board.lan_peer, Some(1), "the box's output goes to it");
        firmware.lan_received(&mut board, 1, b"{\"version\":1,\"event\":\"device.hello\"}\n");
        assert!(board.take_lines().contains(&"MODE DUTY".to_owned()));
    }

    #[test]
    fn a_screenshot_over_the_network_is_the_frame_when_asked_paced_for_the_relay() {
        let (mut firmware, mut board) = paired_box();
        link_up(&mut firmware, &mut board, 1);
        // No two neighbouring pixels alike: the most runs a frame can have.
        let pattern: Vec<u16> = (0..320 * 240).map(|index| (index % 7) as u16 * 0x1111).collect();
        for (index, color) in pattern.iter().enumerate() {
            board.frame[index * 2..index * 2 + 2].copy_from_slice(&color.to_be_bytes());
        }
        firmware.lan_received(&mut board, 1, b"{\"version\":1,\"event\":\"device.screenshot\"}\n");
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
        link_up(&mut firmware, &mut board, 1);
        for line in [
            r#"{"version":1,"event":"device.pair","key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#,
            r#"{"version":1,"event":"device.wifi.set","ssid":"evil","password":"x"}"#,
            r#"{"version":1,"event":"device.relay.set","url":"wss://evil.example"}"#,
        ] {
            firmware.lan_received(&mut board, 1, format!("{line}\n").as_bytes());
            assert_eq!(board.take_lines(), ["ERROR cable_only"]);
        }
    }

    #[test]
    fn the_cable_takes_the_box_back() {
        let (mut firmware, mut board) = paired_box();
        link_up(&mut firmware, &mut board, 1);
        send(&mut firmware, &mut board, r#"{"version":1,"event":"device.heartbeat"}"#);
        assert!(board.lan_closed.contains(&1));
        assert_eq!(board.lan_peer, None);
    }

    #[test]
    fn a_scan_is_asked_for_over_the_cable_only() {
        let (mut firmware, mut board) = paired_box();
        assert!(send(&mut firmware, &mut board, r#"{"version":1,"event":"device.wifi.scan"}"#).is_empty());
        assert!(firmware.take_scan_request());
        assert!(!firmware.take_scan_request(), "once");
        board.advance(USB_QUIET_MS);
        link_up(&mut firmware, &mut board, 1);
        firmware.lan_received(&mut board, 1, b"{\"version\":1,\"event\":\"device.wifi.scan\"}\n");
        assert_eq!(board.take_lines(), ["ERROR cable_only"]);
        assert!(!firmware.take_scan_request());
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
    use vibebuddy_firmware_core::relay_box::{RelayBox, RelayLines, signed_message};

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_box_signs_reports_and_carries_lines() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        send(&mut firmware, &mut board, &format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}","name":"a"}}"#));
        let mut relay = RelayBox::new();

        let hello = parse(&relay.hello(&firmware, "abc").unwrap());
        let box_id = hello["key"].as_str().unwrap().to_owned();
        let key = VerifyingKey::from_bytes(&link::decode_key(&box_id).unwrap()).unwrap();
        let sig = Signature::from_bytes(&link::decode::<64>(hello["sig"].as_str().unwrap()).unwrap());
        assert!(key.verify(&signed_message("box", &box_id, "abc"), &sig).is_ok());
        assert!(relay.report(&firmware, board.now_ms()).is_empty(), "nothing before welcome");

        relay.message(&mut firmware, &mut board, &json!({"t": "welcome"}));
        let reports: Vec<Value> = relay.report(&firmware, board.now_ms()).iter().map(|text| parse(text)).collect();
        assert_eq!(reports[0], json!({"t": "paired", "computers": [{"key": COMPUTER, "name": "a"}]}));
        assert_eq!(reports[1], json!({"t": "linked", "via": "usb", "computer": COMPUTER}), "it was just paired over the cable");
        assert!(relay.report(&firmware, board.now_ms()).is_empty(), "only changes");

        board.advance(USB_QUIET_MS);
        assert_eq!(relay.report(&firmware, board.now_ms()), [r#"{"t":"unlinked"}"#]);

        relay.message(&mut firmware, &mut board, &json!({"t": "linked", "computer": COMPUTER}));
        assert!(relay.has_peer());
        board.take_lines();
        relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": r#"{"version":1,"event":"device.hello"}"#}));
        let output = board.take_lines().join("\n") + "\n";
        let data: Vec<Value> = RelayLines::default().lines(output.as_bytes()).iter().map(|text| parse(text)).collect();
        assert!(data.contains(&json!({"t": "data", "line": "MODE DUTY"})), "{data:?}");

        relay.message(&mut firmware, &mut board, &json!({"t": "unlinked", "reason": "takeover"}));
        assert!(!relay.has_peer());
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
            relay.gone(&mut firmware, board.now_ms());
        }
    }

    #[test]
    fn pairing_over_the_relay_is_refused() {
        let (mut firmware, mut board, _) = booted(blank_flash());
        let mut relay = RelayBox::new();
        relay.hello(&firmware, "n");
        relay.message(&mut firmware, &mut board, &json!({"t": "linked", "computer": COMPUTER}));
        board.take_lines();
        relay.message(&mut firmware, &mut board, &json!({"t": "data", "line": format!(r#"{{"version":1,"event":"device.pair","key":"{COMPUTER}"}}"#)}));
        assert_eq!(board.take_lines(), ["ERROR cable_only"]);
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
