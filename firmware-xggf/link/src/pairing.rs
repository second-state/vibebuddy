//! Pairing and unpairing on the XGGF, with the same rules and replies as the box's firmware
//! (`Firmware::handle_line` and `handle_pairing` in firmware-core). The list has one owner, the bridge's `Link`: C
//! holds no copy and knows no rules, it only stores the bytes it is handed. `tests/pairing_parity.rs` runs the box's
//! firmware beside this and compares every reply.
//!
//! The one deliberate difference is a failed save. The box keeps the change in memory until it restarts, which
//! disagrees with its flash; here the change is undone, so memory and NVS always hold the same list.

use alloc::{format, vec::Vec};
use core::ffi::c_void;
use core::slice;

use serde_json::Value;
use vibebuddy_firmware_core::link::{self, Link, MAX_PAIRED, PairError};
use vibebuddy_protocol::{Event, ProtocolError};

use crate::{Bridge, Callback, emit, outputs};

/// Output kinds, after those in `crate::outputs`: a reply for the cable, and one that also goes to the linked peer,
/// for a line that came over the network.
const KIND_CABLE: u32 = 7;
const KIND_CABLE_AND_PEER: u32 = 8;

/// What C stores: this version, the count, then each computer's key, name length and name. One value, so a save is
/// all or nothing.
const FORMAT: u8 = 1;
const KEY_BYTES: usize = 32;
/// The arrays earlier builds stored under `pairs` and `names`: NUL-terminated key text and name.
const LEGACY_KEY_RECORD: usize = 44;
const LEGACY_NAME_RECORD: usize = 33;

/// Stores the list's bytes; false if it couldn't.
type Save = unsafe extern "C" fn(*mut c_void, *const u8, usize) -> bool;

fn encode(link: &Link) -> Vec<u8> {
    let mut bytes = alloc::vec![FORMAT, link.paired.len() as u8];
    for paired in &link.paired {
        bytes.extend_from_slice(&paired.key);
        bytes.push(paired.name.len() as u8);
        bytes.extend_from_slice(paired.name.as_bytes());
    }
    bytes
}

/// The list in `bytes`; None if they aren't one. A name that isn't text only costs the name: it is a label, and
/// losing the computer over it would lock that computer out of the network.
fn decode(bytes: &[u8]) -> Option<Vec<([u8; KEY_BYTES], &str)>> {
    let (&[version, count], mut rest) = bytes.split_first_chunk::<2>()?;
    if version != FORMAT || count as usize > MAX_PAIRED {
        return None;
    }
    let mut computers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (key, after_key) = rest.split_first_chunk::<KEY_BYTES>()?;
        let (&length, after_length) = after_key.split_first()?;
        let (name, after_name) = after_length.split_at_checked(length as usize)?;
        computers.push((*key, core::str::from_utf8(name).unwrap_or("")));
        rest = after_name;
    }
    rest.is_empty().then_some(computers)
}

fn text_before_nul(record: &[u8]) -> &[u8] {
    record.split(|&byte| byte == 0).next().unwrap_or(&[])
}

/// A line whose event the caller took for a pairing command, handled as the box would. False if it isn't one after all.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn vb_link_pairing(
    bridge: *mut Bridge,
    line: *const u8,
    len: usize,
    from_cable: bool,
    now: u32,
    save: Save,
    save_context: *mut c_void,
    callback: Callback,
    context: *mut c_void,
) -> bool {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return false };
    if line.is_null() {
        return false;
    }
    let kind = if from_cable { KIND_CABLE } else { KIND_CABLE_AND_PEER };
    let say = |text: &str| emit(callback, context, kind, 0, text.as_bytes());

    // As `handle_line` does. The caller's parser accepts bytes this one refuses, such as text that isn't UTF-8.
    let Ok(value) = serde_json::from_slice::<Value>(unsafe { slice::from_raw_parts(line, len) }) else {
        say("ERROR invalid_json");
        return true;
    };
    let title_ok = value.get("title").is_none_or(Value::is_string);
    let parsed = if value.is_object() && title_ok { serde_json::from_value::<Event>(value).ok() } else { None };
    let Some(message) = parsed else {
        say("ERROR invalid_message");
        return true;
    };
    match message.validate() {
        Ok(()) => {}
        Err(ProtocolError::UnsupportedVersion(_)) => {
            say("ERROR unsupported_version");
            return true;
        }
        Err(_) => {
            say("ERROR invalid_message");
            return true;
        }
    }
    let field = |name: &str| message.extra.get(name).and_then(Value::as_str);

    // As `handle_pairing` does. Whoever is at the box decides who may link: only the cable may change that.
    let event = message.event.as_str();
    if !matches!(event, "device.pair" | "device.unpair" | "device.pairs") {
        return false;
    }
    if !from_cable {
        say("ERROR cable_only");
        return true;
    }
    if event == "device.pairs" {
        say(&format!("PAIRS {}", b.link.paired.len()));
        for paired in &b.link.paired {
            let line = format!("PAIR {} {}", link::encode_key(&paired.key), paired.name);
            say(&line.replace(['\r', '\n'], " "));
        }
        return true;
    }
    let text = field("key").unwrap_or("");
    if event == "device.unpair" && text.len() == 43 && link::decode_key(text).is_none() {
        say(&format!("UNPAIRED {text}"));
        return true;
    }
    let Some(key) = link::decode_key(text) else {
        say("PAIR INVALID");
        return true;
    };
    let before = b.link.paired.clone();
    let pairing = event == "device.pair";
    let changed = if pairing {
        match b.link.pair(key, field("name").unwrap_or("")) {
            Ok(changed) => changed,
            Err(PairError::Full) => {
                say("PAIR FULL");
                return true;
            }
        }
    } else {
        b.link.unpair(&key)
    };
    if changed {
        let bytes = encode(&b.link);
        if !unsafe { save(save_context, bytes.as_ptr(), bytes.len()) } {
            b.link.paired = before;
            say("LINK SAVE ERROR");
            return true;
        }
    }
    let label = if pairing { "PAIRED" } else { "UNPAIRED" };
    say(&format!("{label} {}", link::encode_key(&key)));
    if !pairing {
        let result = b.lan.unpaired(&key, now);
        outputs(b, result, callback, context);
    }
    true
}

/// The stored list, at boot. Returns -1 if the bytes aren't a list, 1 if the list was normalized on the way in and
/// should be saved back, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_restore(bridge: *mut Bridge, bytes: *const u8, len: usize) -> i32 {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return -1 };
    if bytes.is_null() {
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(bytes, len) };
    let Some(computers) = decode(bytes) else { return -1 };
    b.link.paired.clear();
    for (key, name) in computers {
        if b.link.pair(key, name).is_err() {
            return -1;
        }
    }
    (encode(&b.link) != bytes) as i32
}

/// The arrays earlier builds stored, at boot. False if `keys` isn't a list of them. A key that can't be read is
/// dropped, since nobody can link with it, and names that don't fit the keys are left blank. Earlier builds could
/// also store a name that isn't text, and then refused to bring Wi-Fi up at all.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_restore_legacy(bridge: *mut Bridge, keys: *const u8, keys_len: usize, names: *const u8, names_len: usize) -> bool {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return false };
    if keys.is_null() || names.is_null() || keys_len % LEGACY_KEY_RECORD != 0 || keys_len > MAX_PAIRED * LEGACY_KEY_RECORD {
        return false;
    }
    let keys = unsafe { slice::from_raw_parts(keys, keys_len) };
    let names = unsafe { slice::from_raw_parts(names, names_len) };
    let count = keys_len / LEGACY_KEY_RECORD;
    let names_fit = names_len == count * LEGACY_NAME_RECORD;
    b.link.paired.clear();
    for (index, record) in keys.chunks_exact(LEGACY_KEY_RECORD).enumerate() {
        let key = core::str::from_utf8(text_before_nul(record)).ok().and_then(link::decode_key);
        let Some(key) = key else { continue };
        let name = if names_fit { &names[index * LEGACY_NAME_RECORD..(index + 1) * LEGACY_NAME_RECORD] } else { &[][..] };
        let name = core::str::from_utf8(text_before_nul(name)).unwrap_or("");
        if b.link.pair(key, name).is_err() {
            return false;
        }
    }
    true
}

/// Saves the list as it is, for after a restore.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_save(bridge: *const Bridge, save: Save, save_context: *mut c_void) -> bool {
    let Some(b) = (unsafe { bridge.as_ref() }) else { return false };
    let bytes = encode(&b.link);
    unsafe { save(save_context, bytes.as_ptr(), bytes.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_paired_count(bridge: *const Bridge) -> u32 {
    unsafe { bridge.as_ref() }.map_or(0, |b| b.link.paired.len() as u32)
}
