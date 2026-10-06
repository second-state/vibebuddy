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
    let mut flash = MemoryFlash { bytes: vec![0xFF; 0x610000] };
    let entries = [("nvs", 1u8, 2u8, 0x9000u32, 0x6000u32), ("phy_init", 1, 1, 0xF000, 0x1000), ("factory", 0, 0, 0x10000, 0x400000), ("voices", 1, 0x40, 0x410000, 0x200000)];
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
}

fn booted(flash: MemoryFlash) -> (Firmware, FakeBoard, Vec<String>) {
    let mut board = FakeBoard::new(flash);
    let mut firmware = Firmware::new(7, board.now_ms(), b"abc1234 2026-09-26 10:00");
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
            "VOICES builtin",
            "AUDIO READY",
            "AUDIO CODEC ES8311",
            "VOLUME 65",
            "BUTTONS READY",
            "READY vibebuddy-fw 0.1.0",
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
    assert_eq!(lines, ["DISPLAY READY BUILD abc1234 2026-09-26 10:00", "MODE DUTY", "VOICES builtin", "VOLUME 65"]);
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
    // VOLUME, MUTE, STATUS, and back to VOLUME.
    press(&mut firmware, &mut board, Key::K1, 1100);
    for _ in 0..3 {
        press(&mut firmware, &mut board, Key::K1, 50);
    }
    assert_eq!(press(&mut firmware, &mut board, Key::K0, 50), ["VOLUME 80", "AUDIO QUEUED DONE"]);
}
