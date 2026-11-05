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

pub mod pairing;

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
    unsafe extern "C" fn saved(_: *mut c_void, _: *const u8, _: usize) -> bool { true }
    fn pair_for_test(bridge: *mut Bridge, key: &str) {
        let line = format!(r#"{{"version":1,"event":"device.pair","key":"{key}","name":"test"}}"#);
        let mut captured = Captured::new();
        assert!(unsafe { pairing::vb_link_pairing(bridge, line.as_ptr(), line.len(), true, 0, saved, core::ptr::null_mut(), collect, (&mut captured as *mut Captured).cast()) });
        assert_eq!(captured, [(7, format!("PAIRED {key}").into_bytes())]);
    }

    #[test]
    fn c_bridge_authenticates_and_carries_encrypted_notifications() {
        let box_seed = [1u8; 32]; let computer_seed = [2u8; 32];
        let bridge = unsafe { vb_link_new(box_seed.as_ptr()) };
        let computer = ed25519_dalek::SigningKey::from_bytes(&computer_seed).verifying_key().to_bytes();
        let key = link::encode_key(&computer);
        pair_for_test(bridge, &key);
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

    /// A bridge with a computer paired and linked over Wi-Fi, and that computer's key.
    fn linked(computer_seed: [u8; 32]) -> (*mut Bridge, alloc::string::String) {
        let bridge = unsafe { vb_link_new([1u8; 32].as_ptr()) };
        let computer = ed25519_dalek::SigningKey::from_bytes(&computer_seed).verifying_key().to_bytes();
        let key = link::encode_key(&computer);
        pair_for_test(bridge, &key);
        let box_key = unsafe { &*bridge }.link.public_key();
        let mut captured = Captured::new();
        let ctx = (&mut captured as *mut Captured).cast();
        unsafe { vb_lan_open(bridge, 1, [4u8; 32].as_ptr(), 100, collect, ctx) };
        let (_, first) = noise::Initiator::start(computer_seed, &box_key, [3; 32], br#"{"takeover":false}"#).unwrap();
        let first = format!("{first}\n");
        captured.clear();
        unsafe { vb_lan_receive(bridge, 1, first.as_ptr(), first.len(), 101, collect, ctx) };
        assert!(captured.iter().any(|event| event.0 == 5), "the computer links");
        (bridge, key)
    }

    #[test]
    fn pairing_over_the_network_is_refused_and_changes_nothing() {
        let (bridge, key) = linked([2; 32]);
        let other = link::encode_key(&[9; 32]);
        for line in [
            format!(r#"{{"version":1,"event":"device.pair","key":"{other}","name":"intruder"}}"#),
            format!(r#"{{"version":1,"event":"device.unpair","key":"{key}"}}"#),
            r#"{"version":1,"event":"device.pairs"}"#.into(),
        ] {
            let mut captured = Captured::new();
            let handled = unsafe { pairing::vb_link_pairing(bridge, line.as_ptr(), line.len(), false, 200, saved, core::ptr::null_mut(), collect, (&mut captured as *mut Captured).cast()) };
            assert!(handled);
            assert_eq!(captured, [(8, b"ERROR cable_only".to_vec())], "{line}");
            assert_eq!(unsafe { pairing::vb_link_paired_count(bridge) }, 1, "{line}");
            assert!(unsafe { &*bridge }.lan.linked_key().is_some(), "the link stays up: {line}");
        }
        unsafe { drop(Box::from_raw(bridge)) };
    }

    #[test]
    fn unpairing_a_linked_computer_tells_it_and_closes_the_link_after_the_reply() {
        let (bridge, key) = linked([2; 32]);
        let mut sent = Captured::new();
        let line = format!(r#"{{"version":1,"event":"device.unpair","key":"{key}"}}"#);
        assert!(unsafe { pairing::vb_link_pairing(bridge, line.as_ptr(), line.len(), true, 200, saved, core::ptr::null_mut(), collect, (&mut sent as *mut Captured).cast()) });
        let kinds: Vec<u32> = sent.iter().map(|event| event.0).collect();
        assert_eq!(sent[0], (7, format!("UNPAIRED {key}").into_bytes()));
        assert_eq!(kinds[1..], [2, 3, 6], "told, closed, and no peer any more");
        assert_eq!(unsafe { pairing::vb_link_paired_count(bridge) }, 0);
        assert!(unsafe { &*bridge }.lan.linked_key().is_none());
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

// Opaque C ABI; the box and XGGF share the same timer implementation.
use vibebuddy_firmware_core::pomodoro::{Pomodoro, Phase, Run, Transition};
#[repr(C)]
pub struct PomoView { pub phase: u32, pub run: u32, pub remaining_ms: u32, pub total_ms: u32, pub completed: u32, pub focus_s: u32 }
#[unsafe(no_mangle)]
pub extern "C" fn vb_pomodoro_new() -> *mut Pomodoro { Box::into_raw(Box::new(Pomodoro::new())) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_free(p: *mut Pomodoro) { if !p.is_null() { drop(unsafe { Box::from_raw(p) }); } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_toggle(p: &mut Pomodoro, now: u32) { p.toggle(now); }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_stop(p: &mut Pomodoro) { p.stop(); }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_tick(p: &mut Pomodoro, now: u32) -> u32 { match p.tick(now) { Transition::Nothing => 0, Transition::FocusEnded => 1, Transition::BreakEnded => 2 } }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_view(p: &Pomodoro, now: u32, out: &mut PomoView) {
 let v=p.view(now); *out=PomoView { phase: if v.phase == Phase::Focus {0} else {1}, run: match v.run { Run::Pending=>0, Run::Running=>1, Run::Paused=>2 }, remaining_ms:v.remaining_ms,total_ms:v.total_ms,completed:v.completed,focus_s:v.focus_s };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_set_day(p: &mut Pomodoro, day:u32) -> bool { p.set_day(day) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_tally(p: &Pomodoro, day:&mut u32, count:&mut u32, secs:&mut u32) { let t=p.tally(); *day=t.day; *count=t.completed; *secs=t.focus_s; }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vb_pomodoro_restore(p: &mut Pomodoro, day:u32, completed:u32, focus_s:u32) { p.restore_tally(vibebuddy_firmware_core::pomodoro::Tally {day,completed,focus_s}); }
#[cfg(test)]
mod pomodoro_ffi_tests {
 use super::*;
 #[test] fn restored_tally_and_abandon_through_c_abi() { unsafe {
     let p=vb_pomodoro_new(); vb_pomodoro_restore(&mut *p,20261103,3,4500);
     let (mut day,mut count,mut seconds)=(0,0,0);
     vb_pomodoro_tally(&*p,&mut day,&mut count,&mut seconds);
     assert_eq!((day,count,seconds),(20261103,3,4500));
     vb_pomodoro_toggle(&mut *p,0); vb_pomodoro_toggle(&mut *p,1000);
     vb_pomodoro_stop(&mut *p); assert!((*p).view(2000).is_idle());
     assert_eq!((*p).tally().completed,3);
     assert!(vb_pomodoro_set_day(&mut *p,20261104));
     assert_eq!((*p).tally().completed,0); vb_pomodoro_free(p);
 } }
 #[cfg(not(feature="fast-clock"))]
 #[test] fn default_build_uses_real_durations() {
     assert_eq!(vibebuddy_firmware_core::TIME_SCALE,1);
     let timer=Pomodoro::new(); assert_eq!(timer.view(0).total_ms,25*60*1000);
 }
 #[test] fn complete_cycle_through_c_abi() { unsafe {
 let p=vb_pomodoro_new(); vb_pomodoro_set_day(&mut *p,20261103);
 let mut v=PomoView {phase:0,run:0,remaining_ms:0,total_ms:0,completed:0,focus_s:0};
 vb_pomodoro_view(&*p,0,&mut v); assert_eq!(v.total_ms,1500000 / vibebuddy_firmware_core::TIME_SCALE);
 vb_pomodoro_toggle(&mut *p,0); assert_eq!(vb_pomodoro_tick(&mut *p,1500000),1);
 assert_eq!(vb_pomodoro_tick(&mut *p,1500000),0);
 vb_pomodoro_toggle(&mut *p,1500000); assert_eq!(vb_pomodoro_tick(&mut *p,1800000),2);
 vb_pomodoro_view(&*p,1800000,&mut v); assert_eq!(v.completed,1); assert_eq!(v.run,0);
 vb_pomodoro_free(p);
 } }
}
