//! The app: QingOS's connection events and the keys go in; lines, mDNS, saved data, sounds and the screen come out.

use qapp::full::{Channel, Keys, key, tone, tone_wav};
use qapp::host;
use serde_json::{Value, json};
use vibegame_core::buddy::{Buddy, News, Sound};
use vibegame_core::firmware::display::{Panel, PanelKind, Screens, Tone};
use vibegame_core::firmware::pomodoro::Tally;
use vibegame_core::frame::{self, BIG_BYTES};
use vibegame_core::identity::{self, Identity};
use vibegame_core::pairing::{Own, OwnScreens};
use vibegame_core::server::{Out, Server};

/// Frames a second: the time is counted in frames, so the app's clock stops with it.
pub const FPS: u64 = 60;
/// START held this long gives up the Pomodoro phase.
const HOLD_FRAMES: u64 = 45;
/// The heartbeat is every 5 s: this long with no line from the computer means no link.
pub const NO_LINK_MS: u64 = 15_000;
/// The box's frame: 320 × 240, two bytes a pixel.
const SMALL_BYTES: usize = 320 * 240 * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Pair,
    Forget,
    Screens,
    Mute,
}

pub const MENU: [MenuItem; 4] = [MenuItem::Pair, MenuItem::Forget, MenuItem::Screens, MenuItem::Mute];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub at: usize,
    /// "Forget computers" asks first.
    pub confirm: bool,
}

pub struct App {
    pub server: Server,
    pub buddy: Buddy,
    own: OwnScreens,
    pub keys: Keys,
    pub frames: u64,
    pub menu: Option<Menu>,
    /// When the linked computer last sent a line; None until a computer links.
    pub heard: Option<u64>,
    pub muted: bool,
    /// QingOS's own mute switch.
    pub system_muted: bool,
    sounds: [Option<Vec<u8>>; 3],
    start_down: Option<u64>,
    /// The screen as it is now, at the box's size, and as last shown at the VibeGame's.
    small: Vec<u8>,
    shown: Vec<u8>,
    big: Vec<u8>,
    fresh: bool,
}

fn random(bytes: &mut [u8]) {
    let drawn = host::random(bytes.len());
    bytes.copy_from_slice(&drawn[..bytes.len()]);
}

/// The VibeGame key, made the first time the app runs.
fn identity() -> Identity {
    if let Some(identity) = host::data_read("identity").and_then(|bytes| Identity::from_saved(&bytes)) {
        return identity;
    }
    let mut seed = [0u8; 32];
    random(&mut seed);
    let _ = host::data_write("identity", &seed);
    Identity::from_seed(seed)
}

fn saved(name: &str) -> Value {
    host::data_read(name).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null)
}

fn tally(saved: &Value) -> Option<Tally> {
    let number = |key: &str| saved[key].as_u64().map(|n| n as u32);
    Some(Tally { day: number("day")?, completed: number("completed")?, focus_s: number("focus_s")? })
}

fn line(label: &[u8], value: &[u8], tone: Tone) -> (Vec<u8>, Vec<u8>, Tone) {
    (label.to_vec(), value.to_vec(), tone)
}

/// A key and what it does now, along the bottom of a panel.
fn hint(key: &'static [u8], action: &'static [u8]) -> (&'static [u8], &'static [u8]) {
    (key, action)
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let computers = host::data_read("computers").map(|bytes| identity::load_computers(&bytes)).unwrap_or_default();
        let settings = saved("settings");
        let muted = settings["muted"].as_bool().unwrap_or(false);
        // The box's screens are the robot's unless the person picked korekore's (ADR-0021).
        let screens = settings["screens"].as_str().and_then(Screens::parse).unwrap_or_default();
        let server = Server::new(identity(), computers, env!("CARGO_PKG_VERSION"), Box::new(random), 0);
        let mut seed = [0u8; 4];
        random(&mut seed);
        let app = Self {
            server,
            buddy: Buddy::new(screens, muted, tally(&saved("tally")), u32::from_le_bytes(seed), 0),
            own: OwnScreens::new(),
            keys: Keys::default(),
            frames: 0,
            menu: None,
            heard: None,
            muted,
            system_muted: false,
            sounds: ["input_required.wav", "done.wav", "failed.wav"].map(host::asset_read),
            start_down: None,
            small: vec![0; SMALL_BYTES],
            shown: vec![0; SMALL_BYTES],
            big: vec![0; BIG_BYTES],
            fresh: true,
        };
        app.answer_mdns();
        app
    }

    /// Milliseconds since the app started, by its frames.
    pub fn now(&self) -> u64 {
        self.frames * 1000 / FPS
    }

    /// The firmware's clock is 32 bits of milliseconds and wraps, as on the box.
    fn now32(&self) -> u32 {
        self.now() as u32
    }

    fn answer_mdns(&self) {
        let identity = self.server.identity();
        let (key, name) = (identity.key(), identity.name());
        let paired = if self.server.computers().is_empty() { "0" } else { "1" };
        host::mdns_set(&name, &[("key", &key), ("name", &name), ("paired", paired)]);
    }

    fn carry_out(&mut self, out: Vec<Out>) {
        for out in out {
            match out {
                Out::Send(id, line) => {
                    host::conn_send(id, &line);
                }
                Out::Close(id) => host::conn_close(id),
                Out::Computers => {
                    let _ = host::data_write("computers", identity::save_computers(self.server.computers()).as_bytes());
                    self.answer_mdns();
                }
                Out::Linked(_) => self.heard = Some(self.now()),
                Out::Unlinked => {}
                Out::Event(event) => {
                    self.heard = Some(self.now());
                    let news = self.buddy.line(&event, self.now32());
                    self.news(news);
                }
            }
        }
    }

    fn news(&mut self, news: News) {
        if news.takes_screen && self.menu.is_some() {
            self.menu = None;
            self.buddy.set_panel(None, self.now32());
        }
        if news.tally {
            let tally = self.buddy.tally();
            let saved = json!({"day": tally.day, "completed": tally.completed, "focus_s": tally.focus_s});
            let _ = host::data_write("tally", saved.to_string().as_bytes());
        }
        if let Some(sound) = news.sound {
            self.play(sound);
        }
    }

    fn play(&self, sound: Sound) {
        if self.muted || self.system_muted {
            return;
        }
        let at = match sound {
            Sound::InputRequired => 0,
            Sound::Done => 1,
            Sound::Failed => 2,
            // A short chime when a Pomodoro phase ends: the robot's voice has no line for it that fits.
            Sound::FocusEnded | Sound::BreakEnded => {
                let hz = if sound == Sound::FocusEnded { 880 } else { 660 };
                tone(Channel::Square1, hz, 180, 40);
                tone(Channel::Square2, hz * 3 / 2, 360, 30);
                return;
            }
        };
        if let Some(wav) = &self.sounds[at] {
            tone_wav(wav);
        }
    }

    pub fn conn_open(&mut self, id: u64) {
        let out = self.server.open(id, self.now());
        self.carry_out(out);
    }

    pub fn conn_line(&mut self, id: u64, text: &str) {
        let out = self.server.line(id, text, self.now());
        self.carry_out(out);
    }

    pub fn conn_close(&mut self, id: u64) {
        let out = self.server.closed(id);
        self.carry_out(out);
    }

    /// The computer said something in the last 15 s. A connection that just closed doesn't count as no link: the
    /// computer connects again within 2 s.
    pub fn link_alive(&self) -> bool {
        self.heard.is_some_and(|heard| self.now().saturating_sub(heard) < NO_LINK_MS)
    }

    fn send(&mut self, event: Value) {
        let out = self.server.send(&event.to_string());
        self.carry_out(out);
    }

    fn save_settings(&self) {
        let screens = if self.buddy.screens() == Screens::Korekore { "korekore" } else { "robot" };
        let _ = host::data_write("settings", json!({"muted": self.muted, "screens": screens}).to_string().as_bytes());
    }

    /// One of the VibeGame's own screens is up in place of the box's.
    fn own_screen(&self) -> bool {
        self.menu.is_none() && (self.server.code().is_some() || self.heard.is_none())
    }

    /// One frame: keys, time, then the screen.
    pub fn step(&mut self, buttons: u32) {
        self.frames += 1;
        self.keys.set(buttons);
        let now = self.now();
        let out = self.server.tick(now);
        self.carry_out(out);
        let news = self.buddy.tick(self.now32(), self.link_alive(), self.menu.is_some() || self.own_screen());
        self.news(news);
        self.keys_pressed();
    }

    fn keys_pressed(&mut self) {
        let k = self.keys;
        let now = self.now32();
        if k.bits() != 0 && [key::A, key::B, key::X, key::Y, key::UP, key::DOWN, key::LEFT, key::RIGHT, key::START, key::SELECT].iter().any(|&key| k.pressed(key)) {
            self.buddy.key(now);
        }
        if self.menu.is_some() {
            return self.menu_keys();
        }
        if self.server.code().is_some() {
            // The pairing screen: B goes back when there are computers already.
            if k.pressed(key::B) {
                self.server.hide_code();
            }
            if k.pressed(key::X) && !self.server.computers().is_empty() {
                self.open_menu(Menu { at: 0, confirm: false });
            }
            return;
        }
        if k.pressed(key::X) {
            return self.open_menu(Menu { at: 0, confirm: false });
        }
        if k.pressed(key::SELECT) {
            self.buddy.switch(now);
        }
        // START: pressed and let go starts, pauses or goes on; held gives up, as K0 on the box.
        if k.pressed(key::START) {
            self.start_down = Some(self.frames);
        }
        if let Some(down) = self.start_down {
            if k.held(key::START) && self.frames - down == HOLD_FRAMES {
                self.buddy.stop(now);
            } else if k.released(key::START) {
                if self.frames - down < HOLD_FRAMES {
                    self.buddy.start(now);
                }
                self.start_down = None;
            }
        }
        if !self.link_alive() {
            return;
        }
        if k.pressed(key::DOWN) || k.pressed(key::UP) {
            self.buddy.pick(k.pressed(key::DOWN), now);
        }
        if k.pressed(key::B) {
            self.buddy.unpick();
        }
        if k.pressed(key::A) {
            match self.buddy.picked() {
                Some(task) => self.send(json!({"version": 1, "event": "focus", "task": task})),
                None => self.send(json!({"version": 1, "event": "button", "button": "K2", "action": "press"})),
            }
            self.buddy.unpick();
        }
    }

    fn open_menu(&mut self, menu: Menu) {
        self.menu = Some(menu);
        let panel = self.panel(menu);
        self.buddy.set_panel(Some(panel), self.now32());
    }

    /// The menu as a panel of the box's own (its menu's look), in either set of screens.
    pub fn panel(&self, menu: Menu) -> Panel {
        let paired = self.server.computers().len().to_string();
        if menu.confirm {
            return Panel {
                kind: PanelKind::Facts,
                title: b"FORGET ALL?".to_vec(),
                lines: vec![line(b"COMPUTERS", paired.as_bytes(), Tone::Plain), line(b"THEN", b"PAIR AGAIN", Tone::Dim)],
                selected: None,
                hints: vec![hint(b"A", b"FORGET"), hint(b"B", b"BACK")],
            };
        }
        let lines = MENU
            .iter()
            .map(|item| match item {
                MenuItem::Pair => line(b"PAIR", b"NEW CODE", Tone::Dim),
                MenuItem::Forget => line(b"FORGET ALL", paired.as_bytes(), Tone::Accent),
                MenuItem::Screens => line(b"SCREENS", self.buddy.screens().name(), Tone::Accent),
                MenuItem::Mute => line(b"MUTE", if self.muted { b"ON" } else { b"OFF" }, Tone::Accent),
            })
            .collect();
        let verb: &'static [u8] = match MENU[menu.at] {
            MenuItem::Pair | MenuItem::Forget => b"OPEN",
            MenuItem::Screens => b"CHANGE",
            MenuItem::Mute => b"TOGGLE",
        };
        Panel { kind: PanelKind::List, title: b"MENU".to_vec(), lines, selected: Some(menu.at), hints: vec![hint(b"A", verb), hint(b"B", b"CLOSE")] }
    }

    fn close_menu(&mut self) {
        self.menu = None;
        self.buddy.set_panel(None, self.now32());
    }

    fn menu_keys(&mut self) {
        let k = self.keys;
        let Some(mut menu) = self.menu else { return };
        let now = self.now32();
        if menu.confirm {
            if k.pressed(key::A) {
                let out = self.server.forget_all(self.now());
                self.carry_out(out);
                self.heard = None;
                self.close_menu();
            } else if k.pressed(key::B) {
                self.open_menu(Menu { confirm: false, ..menu });
            }
            return;
        }
        if k.pressed(key::B) || k.pressed(key::X) {
            return self.close_menu();
        }
        if k.pressed(key::DOWN) {
            menu.at = (menu.at + 1) % MENU.len();
        }
        if k.pressed(key::UP) {
            menu.at = (menu.at + MENU.len() - 1) % MENU.len();
        }
        if k.pressed(key::A) {
            match MENU[menu.at] {
                MenuItem::Pair => {
                    self.server.show_code(self.now());
                    return self.close_menu();
                }
                MenuItem::Forget => menu.confirm = true,
                MenuItem::Screens => {
                    let screens = if self.buddy.screens() == Screens::Korekore { Screens::Robot } else { Screens::Korekore };
                    self.buddy.set_screens(screens, now);
                    self.save_settings();
                }
                MenuItem::Mute => {
                    self.muted = !self.muted;
                    self.buddy.set_muted(self.muted, now);
                    self.save_settings();
                }
            }
        }
        self.open_menu(menu);
    }

    /// Draws the screen as it is now at the box's size; false while it is dark.
    fn compose(&mut self) -> bool {
        let now = self.now32();
        let screens = self.buddy.screens();
        let name = self.server.identity().name();
        let own = match self.server.code() {
            Some(code) if self.own_screen() => Some(Own::Pairing { code, name: &name, back: !self.server.computers().is_empty() }),
            None if self.own_screen() => Some(Own::Looking { name: &name }),
            _ => None,
        };
        if let Some(own) = own {
            self.own.draw(own, screens, now, &mut self.small);
            return true;
        }
        match self.buddy.frame() {
            Some(frame) => {
                self.small.copy_from_slice(frame);
                true
            }
            None => {
                self.small.fill(0);
                false
            }
        }
    }

    /// The screen at the VibeGame's size (640 × 480 RGB565, high byte first), when it changed since the last call;
    /// None when it is the same: QingOS keeps showing the last frame.
    pub fn picture(&mut self) -> Option<&[u8]> {
        let lit = self.compose();
        if !self.fresh && self.small == self.shown {
            return None;
        }
        self.fresh = false;
        self.shown.copy_from_slice(&self.small);
        frame::double(lit.then_some(&self.small[..]), &mut self.big);
        Some(&self.big)
    }

    /// The screen at the VibeGame's size as it is now, changed or not: for screenshots.
    pub fn shot(&mut self) -> &[u8] {
        let lit = self.compose();
        frame::double(lit.then_some(&self.small[..]), &mut self.big);
        &self.big
    }
}
