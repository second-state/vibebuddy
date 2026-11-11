//! VibeBuddy on the VibeGame handheld: a full-screen QingOS app (docs/vibegame.md). The computer connects to it
//! on the local network and sends the same task cards, stats and announcements the box gets.
//!
//! - [`app`]: connections, keys, sounds and saved data.
//! - [`screens`] and [`art`]: what is drawn.
//! - `vibegame-core` (`core/`): pairing, the link and the events, without QingOS.

pub mod app;
pub mod art;
pub mod screens;

use std::sync::Mutex;

use qapp::full::{Frame, buttons};
use qapp::{Conn, Event};

use crate::app::App;
use crate::art::Art;

struct Running {
    app: App,
    art: Art,
    frame: Frame,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Running) -> R) -> R {
    let mut running = RUNNING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(running.get_or_insert_with(|| Running { app: App::new(), art: Art::new(), frame: Frame::default() }))
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
            screens::draw(&r.app, &r.art, &mut r.frame);
            r.frame.show();
        })
    });
});
