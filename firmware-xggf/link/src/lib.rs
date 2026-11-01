#![cfg_attr(target_os = "none", no_std)]

extern crate alloc;
use alloc::boxed::Box;
#[cfg(target_os = "none")]
use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
#[cfg(target_os = "none")]
use core::ffi::c_int;
#[cfg(target_os = "none")]
use core::panic::PanicInfo;
use core::slice;
use vibebuddy_firmware_core::{lan, link, noise};

#[cfg(target_os = "none")]
unsafe extern "C" {
    fn posix_memalign(ptr: *mut *mut c_void, alignment: usize, size: usize) -> c_int;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

#[cfg(target_os = "none")]
struct IdAllocator;
#[cfg(target_os = "none")]
unsafe impl GlobalAlloc for IdAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut ptr = core::ptr::null_mut();
        let align = layout.align().max(core::mem::size_of::<usize>());
        if unsafe { posix_memalign(&mut ptr, align, layout.size().max(1)) } != 0 { return core::ptr::null_mut(); }
        ptr.cast()
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) { unsafe { free(ptr.cast()) } }
}
#[cfg(target_os = "none")]
#[global_allocator]
static ALLOCATOR: IdAllocator = IdAllocator;

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &PanicInfo) -> ! { unsafe { abort() } }

pub struct Bridge { link: link::Link, session: Option<noise::Accepted>, lan: lan::Lan, sealer: Option<noise::SealedLines> }

fn copy(bytes: &[u8], out: *mut u8, cap: usize) -> isize {
    if out.is_null() || bytes.len() > cap { return -1; }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    bytes.len() as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_new(seed: *const u8) -> *mut Bridge {
    if seed.is_null() { return core::ptr::null_mut(); }
    let mut bytes = [0; 32]; bytes.copy_from_slice(unsafe { slice::from_raw_parts(seed, 32) });
    Box::into_raw(Box::new(Bridge { link: link::Link::new(bytes), session: None, lan: lan::Lan::new(), sealer: None }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_pair(bridge: *mut Bridge, key: *const u8, key_len: usize, name: *const u8, name_len: usize) -> bool {
    let Some(bridge) = (unsafe { bridge.as_mut() }) else { return false };
    if key.is_null() { return false; }
    let Some(key) = core::str::from_utf8(unsafe { slice::from_raw_parts(key, key_len) }).ok().and_then(link::decode_key) else { return false };
    if name.is_null() { return false; }
    let Ok(name) = core::str::from_utf8(unsafe { slice::from_raw_parts(name, name_len) }) else { return false };
    bridge.link.pair(key, name).is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_valid_key(key: *const u8, len: usize) -> bool {
    !key.is_null() && core::str::from_utf8(unsafe { slice::from_raw_parts(key, len) }).ok().and_then(link::decode_key).is_some()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_unpair(bridge: *mut Bridge, key: *const u8, key_len: usize, now: u32, callback: Callback, context: *mut c_void) -> bool {
    let Some(bridge) = (unsafe { bridge.as_mut() }) else { return false };
    if key.is_null() { return false; }
    let Some(key) = core::str::from_utf8(unsafe { slice::from_raw_parts(key, key_len) }).ok().and_then(link::decode_key) else { return false };
    let changed = bridge.link.unpair(&key);
    let result = bridge.lan.unpaired(&key, now); outputs(bridge, result, callback, context);
    changed
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_public(bridge: *const Bridge, out: *mut u8, cap: usize) -> isize {
    let Some(bridge) = (unsafe { bridge.as_ref() }) else { return -1 };
    copy(link::encode_key(&bridge.link.public_key()).as_bytes(), out, cap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_accept(bridge: *mut Bridge, ephemeral: *const u8, input: *const u8, len: usize, out: *mut u8, cap: usize) -> isize {
    let Some(bridge) = (unsafe { bridge.as_mut() }) else { return -1 };
    if ephemeral.is_null() || input.is_null() { return -1; }
    let mut secret = [0; 32]; secret.copy_from_slice(unsafe { slice::from_raw_parts(ephemeral, 32) });
    bridge.session = None;
    let Ok(accepted) = noise::accept(&bridge.link, secret, unsafe { slice::from_raw_parts(input, len) }) else { return -1 };
    let written = copy(accepted.reply.as_bytes(), out, cap);
    if written >= 0 { bridge.session = Some(accepted); }
    written
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_open(bridge: *mut Bridge, input: *const u8, len: usize, out: *mut u8, cap: usize) -> isize {
    let Some(session) = (unsafe { bridge.as_mut() }).and_then(|b| b.session.as_mut()) else { return -1 };
    if input.is_null() { return -1; }
    let Some(plain) = session.opener.open(unsafe { slice::from_raw_parts(input, len) }) else { return -1 };
    copy(&plain, out, cap)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_link_seal(bridge: *mut Bridge, input: *const u8, len: usize, out: *mut u8, cap: usize) -> isize {
    let Some(session) = (unsafe { bridge.as_mut() }).and_then(|b| b.session.as_mut()) else { return -1 };
    if input.is_null() { return -1; }
    let Some(sealed) = session.sealer.seal(unsafe { slice::from_raw_parts(input, len) }) else { return -1 };
    copy(sealed.as_bytes(), out, cap)
}

type Callback = unsafe extern "C" fn(*mut c_void, u32, u32, *const u8, usize);
fn emit(callback: Callback, context: *mut c_void, kind: u32, conn: u32, bytes: &[u8]) {
    unsafe { callback(context, kind, conn, bytes.as_ptr(), bytes.len()) }
}
fn outputs(bridge: &mut Bridge, items: alloc::vec::Vec<lan::Output>, callback: Callback, context: *mut c_void) {
    for item in items {
        match item {
            lan::Output::Send(conn, text) => emit(callback, context, 1, conn, text.as_bytes()),
            lan::Output::SendPeer(text) => {
                if let Some(line) = bridge.sealer.as_mut().and_then(|s| s.seal(text.as_bytes())) {
                    emit(callback, context, 2, 0, line.as_bytes());
                }
            }
            lan::Output::Close(conn) => emit(callback, context, 3, conn, &[]),
            lan::Output::Line(bytes) => emit(callback, context, 4, 0, &bytes),
            lan::Output::Peer(Some(conn)) => {
                bridge.sealer = bridge.lan.take_sealer(conn).map(noise::SealedLines::new);
                emit(callback, context, 5, conn, &[]);
            }
            lan::Output::Peer(None) => { bridge.sealer = None; emit(callback, context, 6, 0, &[]); }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_open(bridge: *mut Bridge, conn: u32, ephemeral: *const u8, now: u32, callback: Callback, context: *mut c_void) {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return };
    if ephemeral.is_null() { return; }
    let mut secret = [0; 32]; secret.copy_from_slice(unsafe { slice::from_raw_parts(ephemeral, 32) });
    let result = b.lan.opened(&b.link, conn, secret, now); outputs(b, result, callback, context);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_receive(bridge: *mut Bridge, conn: u32, input: *const u8, len: usize, now: u32, callback: Callback, context: *mut c_void) {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return };
    if input.is_null() { return; }
    let result = b.lan.received(&b.link, conn, unsafe { slice::from_raw_parts(input, len) }, now); outputs(b, result, callback, context);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_closed(bridge: *mut Bridge, conn: u32, now: u32, callback: Callback, context: *mut c_void) {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return };
    let result = b.lan.closed(conn, now); outputs(b, result, callback, context);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_usb(bridge: *mut Bridge, now: u32, callback: Callback, context: *mut c_void) {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return };
    let result = b.lan.usb_line(now); outputs(b, result, callback, context);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_poll(bridge: *mut Bridge, now: u32, callback: Callback, context: *mut c_void) {
    let Some(b) = (unsafe { bridge.as_mut() }) else { return };
    let result = b.lan.poll(now); outputs(b, result, callback, context);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_lan_send(bridge: *mut Bridge, input: *const u8, len: usize, out: *mut u8, cap: usize) -> isize {
    let Some(sealer) = (unsafe { bridge.as_mut() }).and_then(|b| b.sealer.as_mut()) else { return -1 };
    if input.is_null() || noise::sealed_len(len) > cap { return -1; }
    let Some(line) = sealer.seal(unsafe { slice::from_raw_parts(input, len) }) else { return -1 };
    copy(line.as_bytes(), out, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, vec::Vec};
    type Captured = Vec<(u32, Vec<u8>)>;
    unsafe extern "C" fn collect(ctx: *mut c_void, kind: u32, _: u32, data: *const u8, len: usize) {
        let list = unsafe { &mut *(ctx as *mut Captured) };
        list.push((kind, unsafe { slice::from_raw_parts(data, len) }.into()));
    }

    #[test]
    fn c_bridge_authenticates_and_carries_encrypted_notifications() {
        let box_seed = [1u8; 32]; let computer_seed = [2u8; 32];
        let bridge = unsafe { vb_link_new(box_seed.as_ptr()) };
        let computer = ed25519_dalek::SigningKey::from_bytes(&computer_seed).verifying_key().to_bytes();
        let key = link::encode_key(&computer);
        assert!(unsafe { vb_link_pair(bridge, key.as_ptr(), key.len(), b"test".as_ptr(), 4) });
        let box_key = unsafe { &*bridge }.link.public_key();
        let mut captured = Captured::new(); let ctx = (&mut captured as *mut Captured).cast();
        unsafe { vb_lan_open(bridge, 1, [4u8;32].as_ptr(), 100, collect, ctx) };
        assert_eq!(captured[0].0, 1);
        let (initiator, first) = noise::Initiator::start(computer_seed, &box_key, [3;32], br#"{"takeover":false}"#).unwrap();
        let first = format!("{first}\n"); captured.clear();
        unsafe { vb_lan_receive(bridge, 1, first.as_ptr(), first.len(), 101, collect, ctx) };
        assert_eq!(captured.iter().map(|e| e.0).collect::<Vec<_>>(), [1, 1, 5]);
        let (mut sealer, mut opener) = initiator.finish(&captured[0].1).unwrap();
        assert_eq!(opener.open(&captured[1].1).unwrap(), br#"{"version":1,"event":"link.welcome"}"#);
        let event = br#"{"version":1,"event":"task.start"}"#;
        let encrypted = format!("{}\n", sealer.seal(event).unwrap()); captured.clear();
        unsafe { vb_lan_receive(bridge, 1, encrypted.as_ptr(), encrypted.len(), 102, collect, ctx) };
        assert_eq!(captured, [(4, event.to_vec())]);
        captured.clear(); unsafe { vb_lan_usb(bridge, 103, collect, ctx) };
        assert!(captured.iter().any(|e| e.0 == 3), "USB preempts the network");
        captured.clear();
        unsafe { vb_lan_receive(bridge, 1, encrypted.as_ptr(), encrypted.len(), 104, collect, ctx) };
        assert!(!captured.iter().any(|e| e.0 == 4), "a closed peer cannot deliver notifications");
        unsafe { vb_lan_open(bridge, 2, [6u8;32].as_ptr(), 20000, collect, ctx) };
        let (initiator, first) = noise::Initiator::start(computer_seed, &box_key, [7;32], br#"{"takeover":false}"#).unwrap();
        let first = format!("{first}\n"); captured.clear();
        unsafe { vb_lan_receive(bridge, 2, first.as_ptr(), first.len(), 20001, collect, ctx) };
        let (mut sealer, _) = initiator.finish(&captured[0].1).unwrap();
        let encrypted = format!("{}\n", sealer.seal(event).unwrap()); captured.clear();
        unsafe { vb_lan_receive(bridge, 2, encrypted.as_ptr(), encrypted.len(), 20002, collect, ctx) };
        assert_eq!(captured, [(4, event.to_vec())]); captured.clear();
        unsafe { vb_lan_receive(bridge, 2, encrypted.as_ptr(), encrypted.len(), 20003, collect, ctx) };
        assert!(captured.iter().any(|e| e.0 == 3), "a replay closes the connection");
        unsafe { drop(Box::from_raw(bridge)) };
    }

    #[test]
    fn unpaired_computer_is_refused() {
        let bridge = unsafe { vb_link_new([1u8;32].as_ptr()) };
        let box_key = unsafe { &*bridge }.link.public_key();
        let mut captured = Captured::new(); let ctx = (&mut captured as *mut Captured).cast();
        unsafe { vb_lan_open(bridge, 1, [4u8;32].as_ptr(), 100, collect, ctx) };
        let (_, first) = noise::Initiator::start([2;32], &box_key, [3;32], br#"{"takeover":true}"#).unwrap();
        let first = format!("{first}\n"); captured.clear();
        unsafe { vb_lan_receive(bridge, 1, first.as_ptr(), first.len(), 101, collect, ctx) };
        assert!(captured.iter().any(|e| e.0 == 3));
        assert!(!captured.iter().any(|e| e.0 == 5));
        unsafe { drop(Box::from_raw(bridge)) };
    }
}
