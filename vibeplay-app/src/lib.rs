//! VibeBuddy on the VibePlay handheld: a full-screen QingOS app (docs/vibeplay.md). The computer connects to it
//! on the local network and sends the same task cards, stats and announcements the box gets, and the app draws them
//! with the box's own code.
//!
//! - [`app`]: connections, keys, the menu, sounds and saved data.
//! - `vibeplay-core` (`core/`): pairing, the link, and the box's screens, without QingOS.

pub mod app;

use std::sync::Mutex;

use qapp::full::{Frame565, buttons};
use qapp::{Conn, Event};

use crate::app::App;

struct Running {
    app: App,
    frame: Frame565,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Running) -> R) -> R {
    let mut running = RUNNING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(running.get_or_insert_with(|| Running { app: App::new(), frame: Frame565::default() }))
}

qapp::main!(|app| {
    // Made now, not on the first frame, so mDNS has the key and name from the start.
    with(|_| {});
    app.on(|event| {
        with(|r| match event {
            Event::Conn { id, what: Conn::Open { .. } } => r.app.conn_open(id),
            Event::Conn { id, what: Conn::Line(text) } => r.app.conn_line(id, &text),
            Event::Conn { id, what: Conn::Close } => r.app.conn_close(id),
            Event::Mute { on } => r.app.system_muted = on,
            _ => {}
        })
    });
    app.every(|| {
        with(|r| {
            r.app.step(buttons());
            // Only a frame that changed goes to QingOS, which keeps showing the last one (ADR 0036).
            if let Some(picture) = r.app.picture() {
                r.frame.pixels.copy_from_slice(picture);
                r.frame.show();
            }
        })
    });
});
