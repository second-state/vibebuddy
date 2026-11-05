//! The XGGF's pairing against the box's: the box's real firmware runs on a fake board beside the bridge, and every
//! line gets the same reply from both. Nothing here says what the replies should be, so a change to either side that
//! the other doesn't make fails here.

use core::ffi::c_void;
use core::slice;
use std::cell::Cell;

use vibebuddy_firmware_core::buttons::Levels;
use vibebuddy_firmware_core::audio::Sound;
use vibebuddy_firmware_core::canvas::FRAME_BYTES;
use vibebuddy_firmware_core::display::Screen;
use vibebuddy_firmware_core::firmware::{AudioStatus, Board, Firmware, FrameAction, VolumeError};
use vibebuddy_firmware_core::link;
use vibebuddy_firmware_core::storage::{Flash, FlashError};
use vibebuddy_xggf_link::pairing::{vb_link_pairing, vb_link_paired_count, vb_link_restore, vb_link_restore_legacy, vb_link_save};
use vibebuddy_xggf_link::{Bridge, vb_link_new};

struct MemoryFlash(Vec<u8>);

impl Flash for MemoryFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), FlashError> {
        let start = offset as usize;
        bytes.copy_from_slice(self.0.get(start..start + bytes.len()).ok_or(FlashError)?);
        Ok(())
    }
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), FlashError> {
        let start = offset as usize;
        for (cell, &byte) in self.0.get_mut(start..start + bytes.len()).ok_or(FlashError)?.iter_mut().zip(bytes) {
            *cell &= byte;
        }
        Ok(())
    }
    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        self.0.get_mut(from as usize..to as usize).ok_or(FlashError)?.fill(0xFF);
        Ok(())
    }
}

/// The box's flash, with the partition table its firmware expects (firmware-rs/core/tests/firmware.rs).
fn blank_flash() -> MemoryFlash {
    let mut flash = MemoryFlash(vec![0xFF; 0xA14000]);
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
        let entry = &mut flash.0[at..at + 32];
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

struct TheBox {
    now: Cell<u32>,
    output: Vec<u8>,
    flash: MemoryFlash,
    frame: Vec<u8>,
}

impl Screen for TheBox {
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

impl Board for TheBox {
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
        let (frame, output) = (&self.frame, &mut self.output);
        action(frame, &mut |bytes| output.extend_from_slice(bytes));
    }
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
    fn random(&mut self) -> Option<[u8; 32]> {
        Some([0x5a; 32])
    }
}

fn take_lines(board: &mut TheBox) -> Vec<String> {
    let text = String::from_utf8(std::mem::take(&mut board.output)).expect("the box writes UTF-8");
    text.lines().map(str::to_owned).collect()
}

fn boot(flash: MemoryFlash) -> (Firmware, TheBox) {
    let mut board = TheBox { now: Cell::new(1000), output: Vec::new(), flash, frame: vec![0; FRAME_BYTES] };
    let mut firmware = Firmware::new(7, board.now_ms(), b"parity", "1.0.0");
    firmware.boot(&mut board);
    take_lines(&mut board);
    (firmware, board)
}

fn ask_box(firmware: &mut Firmware, board: &mut TheBox, line: &[u8]) -> Vec<String> {
    firmware.receive(board, line);
    firmware.receive(board, b"\n");
    take_lines(board)
}

/// What C's side of the bridge does: stores what it's handed, unless told to fail, and collects the replies.
#[derive(Default)]
struct Store {
    bytes: Vec<u8>,
    saves: usize,
    failing: bool,
}

unsafe extern "C" fn save(context: *mut c_void, bytes: *const u8, len: usize) -> bool {
    let store = unsafe { &mut *(context as *mut Store) };
    if store.failing {
        return false;
    }
    store.bytes = unsafe { slice::from_raw_parts(bytes, len) }.to_vec();
    store.saves += 1;
    true
}

unsafe extern "C" fn collect(context: *mut c_void, kind: u32, _: u32, bytes: *const u8, len: usize) {
    let lines = unsafe { &mut *(context as *mut Vec<String>) };
    assert_eq!(kind, 7, "no connection is open, so a reply is all there can be");
    lines.push(String::from_utf8(unsafe { slice::from_raw_parts(bytes, len) }.to_vec()).expect("replies are UTF-8"));
}

fn new_bridge() -> *mut Bridge {
    unsafe { vb_link_new([1u8; 32].as_ptr()) }
}

fn ask_bridge(bridge: *mut Bridge, store: &mut Store, line: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let handled = unsafe {
        vb_link_pairing(bridge, line.as_ptr(), line.len(), true, 2000, save, (store as *mut Store).cast(), collect, (&mut lines as *mut Vec<String>).cast())
    };
    assert!(handled, "{}", String::from_utf8_lossy(line));
    lines
}

fn key(byte: u8) -> String {
    link::encode_key(&[byte; 32])
}

fn pair(key: &str, name: &str) -> Vec<u8> {
    format!(r#"{{"version":1,"event":"device.pair","key":"{key}","name":"{name}"}}"#).into_bytes()
}

fn unpair(key: &str) -> Vec<u8> {
    format!(r#"{{"version":1,"event":"device.unpair","key":"{key}"}}"#).into_bytes()
}

const PAIRS: &[u8] = br#"{"version":1,"event":"device.pairs"}"#;

/// Every line, in order, so each also runs against the list the ones before it left.
fn script() -> Vec<Vec<u8>> {
    let a = key(2);
    let b = key(3);
    let mut bad_name = br#"{"version":1,"event":"device.pair","key":"#.to_vec();
    bad_name.extend_from_slice(format!(r#""{a}","name":"bad"#).as_bytes());
    bad_name.push(0xff);
    bad_name.extend_from_slice(br#""}"#);
    let mut lines = vec![
        PAIRS.to_vec(),
        // Pairing, unchanged and renamed.
        pair(&a, "desk"),
        pair(&a, "desk"),
        pair(&a, "renamed"),
        PAIRS.to_vec(),
        // Names: absent, not a string, too long, cut inside a character, with line breaks, with a NUL.
        format!(r#"{{"version":1,"event":"device.pair","key":"{b}"}}"#).into_bytes(),
        format!(r#"{{"version":1,"event":"device.pair","key":"{b}","name":7}}"#).into_bytes(),
        pair(&b, &"x".repeat(32)),
        PAIRS.to_vec(),
        pair(&b, &"y".repeat(40)),
        PAIRS.to_vec(),
        pair(&b, &"é".repeat(20)),
        PAIRS.to_vec(),
        pair(&b, &"界".repeat(12)),
        PAIRS.to_vec(),
        pair(&b, r"line\nbreak\rhere"),
        PAIRS.to_vec(),
        pair(&b, r"before\u0000after"),
        PAIRS.to_vec(),
        // The line that used to be saved and then stop Wi-Fi at every boot: a name that isn't UTF-8.
        bad_name,
        PAIRS.to_vec(),
        // Keys: absent, not a string, too short, too long, a bad character, not canonical.
        br#"{"version":1,"event":"device.pair","name":"x"}"#.to_vec(),
        br#"{"version":1,"event":"device.pair","key":7}"#.to_vec(),
        pair("short", "x"),
        pair(&format!("{a}A"), "x"),
        pair(&format!("{}!", &a[..42]), "x"),
        pair(&format!("{}J", &a[..42]), "x"),
        br#"{"version":1,"event":"device.unpair"}"#.to_vec(),
        br#"{"version":1,"event":"device.unpair","key":7}"#.to_vec(),
        unpair("short"),
        unpair(&format!("{}!", &a[..42])),
        unpair(&format!("{}J", &a[..42])),
        // The envelope: the box's parser decides these, so the bridge must too.
        format!(r#"{{"version":1,"event":"device.pair","key":"{b}","title":3}}"#).into_bytes(),
        format!(r#"{{"version":1,"event":"device.pair","key":"{b}","title":"ok"}}"#).into_bytes(),
        format!(r#"{{"version":1,"event":"device.pair","key":"{b}","id":3}}"#).into_bytes(),
        format!(r#"{{"version":1.0,"event":"device.pair","key":"{b}"}}"#).into_bytes(),
        format!(r#"{{"version":1,"event":"device.pair","key":"{a}","key":"{b}","name":"twice"}}"#).into_bytes(),
        PAIRS.to_vec(),
        // Unpairing: one that isn't there, then both, then again.
        unpair(&key(9)),
        unpair(&a),
        PAIRS.to_vec(),
        unpair(&b),
        unpair(&b),
        PAIRS.to_vec(),
    ];
    // The limit, and that one already there can still be renamed at it.
    for byte in 10..26 {
        lines.push(pair(&key(byte), &format!("computer {byte}")));
    }
    lines.push(PAIRS.to_vec());
    lines.push(pair(&key(99), "one too many"));
    lines.push(pair(&key(10), "renamed when full"));
    lines.push(unpair(&key(15)));
    lines.push(pair(&key(99), "fits now"));
    lines.push(PAIRS.to_vec());
    lines
}

#[test]
fn every_reply_matches_the_boxs_and_so_does_the_list_after_a_restart() {
    let (mut firmware, mut board) = boot(blank_flash());
    let bridge = new_bridge();
    let mut store = Store::default();
    for line in script() {
        let expected = ask_box(&mut firmware, &mut board, &line);
        let actual = ask_bridge(bridge, &mut store, &line);
        assert_eq!(actual, expected, "for {}", String::from_utf8_lossy(&line));
    }

    let (mut firmware, mut board) = boot(board.flash);
    let restarted = new_bridge();
    assert_eq!(unsafe { vb_link_restore(restarted, store.bytes.as_ptr(), store.bytes.len()) }, 0);
    let expected = ask_box(&mut firmware, &mut board, PAIRS);
    assert_eq!(expected[0], "PAIRS 16");
    assert_eq!(ask_bridge(restarted, &mut store, PAIRS), expected);
}

#[test]
fn storage_is_only_touched_when_the_list_changes() {
    let bridge = new_bridge();
    let mut store = Store::default();
    let a = key(2);
    ask_bridge(bridge, &mut store, &pair(&a, "desk"));
    assert_eq!(store.saves, 1);
    ask_bridge(bridge, &mut store, &pair(&a, "desk"));
    ask_bridge(bridge, &mut store, PAIRS);
    ask_bridge(bridge, &mut store, &unpair(&key(9)));
    ask_bridge(bridge, &mut store, &pair("short", "x"));
    assert_eq!(store.saves, 1, "nothing changed, so nothing is written");
    ask_bridge(bridge, &mut store, &pair(&a, "renamed"));
    ask_bridge(bridge, &mut store, &unpair(&a));
    assert_eq!(store.saves, 3);
}

#[test]
fn a_failed_save_leaves_the_list_as_it_was() {
    let bridge = new_bridge();
    let mut store = Store::default();
    let a = key(2);
    ask_bridge(bridge, &mut store, &pair(&a, "desk"));
    let before = ask_bridge(bridge, &mut store, PAIRS);
    let stored = store.bytes.clone();

    store.failing = true;
    for line in [pair(&key(3), "new"), pair(&a, "renamed"), unpair(&a)] {
        assert_eq!(ask_bridge(bridge, &mut store, &line), ["LINK SAVE ERROR"]);
        assert_eq!(ask_bridge(bridge, &mut store, PAIRS), before, "memory stays what storage holds");
    }
    assert_eq!(store.bytes, stored);

    store.failing = false;
    assert_eq!(ask_bridge(bridge, &mut store, &unpair(&a)), [format!("UNPAIRED {a}")]);
    assert_eq!(ask_bridge(bridge, &mut store, PAIRS), ["PAIRS 0"]);
}

#[test]
fn removing_the_last_computer_stores_an_empty_list() {
    let bridge = new_bridge();
    let mut store = Store::default();
    let a = key(2);
    ask_bridge(bridge, &mut store, &pair(&a, "desk"));
    ask_bridge(bridge, &mut store, &unpair(&a));
    assert_eq!(store.bytes, [1, 0]);
    let restarted = new_bridge();
    assert_eq!(unsafe { vb_link_restore(restarted, store.bytes.as_ptr(), store.bytes.len()) }, 0);
    assert_eq!(unsafe { vb_link_paired_count(restarted) }, 0);
}

#[test]
fn stored_bytes_that_are_not_a_list_are_refused() {
    let good = {
        let bridge = new_bridge();
        let mut store = Store::default();
        ask_bridge(bridge, &mut store, &pair(&key(2), "desk"));
        store.bytes
    };
    let mut wrong_version = good.clone();
    wrong_version[0] = 2;
    let mut count_too_high = good.clone();
    count_too_high[1] = 17;
    let mut count_too_low = good.clone();
    count_too_low[1] = 0;
    let mut name_too_long = good.clone();
    name_too_long[34] = 200;
    let mut extra = good.clone();
    extra.push(0);
    for bytes in [vec![], vec![1], wrong_version, count_too_high, count_too_low, name_too_long, extra, good[..good.len() - 1].to_vec()] {
        let bridge = new_bridge();
        assert_eq!(unsafe { vb_link_restore(bridge, bytes.as_ptr(), bytes.len()) }, -1, "{bytes:?}");
        assert_eq!(unsafe { vb_link_paired_count(bridge) }, 0);
    }
}

#[test]
fn a_stored_name_that_is_not_text_costs_the_name_and_not_the_computer() {
    let a = [2u8; 32];
    let mut bytes = vec![1, 1];
    bytes.extend_from_slice(&a);
    bytes.extend_from_slice(&[3, b'b', b'a', 0xff]);
    let bridge = new_bridge();
    assert_eq!(unsafe { vb_link_restore(bridge, bytes.as_ptr(), bytes.len()) }, 1, "normalized, so saved back");
    let mut store = Store::default();
    assert_eq!(ask_bridge(bridge, &mut store, PAIRS), ["PAIRS 1".to_owned(), format!("PAIR {} ", key(2))]);
}

/// The arrays earlier builds stored: 44 bytes of key text and 33 of name, each NUL-terminated.
fn legacy(entries: &[(&[u8], &[u8])]) -> (Vec<u8>, Vec<u8>) {
    let (mut keys, mut names) = (Vec::new(), Vec::new());
    for (key, name) in entries {
        let mut record = key.to_vec();
        record.resize(44, 0);
        keys.extend_from_slice(&record);
        let mut record = name.to_vec();
        record.resize(33, 0);
        names.extend_from_slice(&record);
    }
    (keys, names)
}

fn restore_legacy(keys: &[u8], names: &[u8]) -> (bool, *mut Bridge) {
    let bridge = new_bridge();
    let keys_pointer = if keys.is_empty() { [0u8; 1].as_ptr() } else { keys.as_ptr() };
    let names_pointer = if names.is_empty() { [0u8; 1].as_ptr() } else { names.as_ptr() };
    (unsafe { vb_link_restore_legacy(bridge, keys_pointer, keys.len(), names_pointer, names.len()) }, bridge)
}

#[test]
fn the_earlier_format_is_read_and_a_bad_name_no_longer_stops_wifi() {
    let (a, b) = (key(2), key(3));
    let (keys, names) = legacy(&[(a.as_bytes(), b"desk"), (b"not a key", b"gone"), (b.as_bytes(), b"bad\xff")]);
    let (restored, bridge) = restore_legacy(&keys, &names);
    assert!(restored);
    let mut store = Store::default();
    assert_eq!(
        ask_bridge(bridge, &mut store, PAIRS),
        ["PAIRS 2".to_owned(), format!("PAIR {a} desk"), format!("PAIR {b} ")],
        "the bad key is dropped, and the bad name costs only the name"
    );

    // It moves to the new format whole: a restart from what's saved gives the same list.
    assert!(unsafe { vb_link_save(bridge, save, (&mut store as *mut Store).cast()) });
    let restarted = new_bridge();
    assert_eq!(unsafe { vb_link_restore(restarted, store.bytes.as_ptr(), store.bytes.len()) }, 0);
    assert_eq!(ask_bridge(restarted, &mut store, PAIRS), ["PAIRS 2".to_owned(), format!("PAIR {a} desk"), format!("PAIR {b} ")]);
}

#[test]
fn names_that_do_not_fit_the_earlier_keys_are_left_blank() {
    let a = key(2);
    let (keys, names) = legacy(&[(a.as_bytes(), b"desk")]);
    for names in [&[][..], &names[..32]] {
        let (restored, bridge) = restore_legacy(&keys, names);
        assert!(restored);
        let mut store = Store::default();
        assert_eq!(ask_bridge(bridge, &mut store, PAIRS), ["PAIRS 1".to_owned(), format!("PAIR {a} ")]);
    }
}

#[test]
fn earlier_keys_that_are_not_a_list_are_refused() {
    let (keys, names) = legacy(&[(key(2).as_bytes(), b"desk")]);
    assert!(!restore_legacy(&keys[..43], &names).0);
    let too_many = vec![0u8; 44 * 17];
    assert!(!restore_legacy(&too_many, &[]).0);
}
