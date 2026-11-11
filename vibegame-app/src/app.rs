//! The app: QingOS's connection events and the keys go in; lines, mDNS, saved data, sounds and the screen come out.

use qapp::full::{Channel, Keys, key, tone, tone_wav};
use qapp::host;
use serde_json::{Value, json};
use vibegame_core::duty::{Duty, Sound};
use vibegame_core::identity::{self, Identity};
use vibegame_core::pomodoro::{Phase, Pomodoro};
use vibegame_core::server::{Out, Server};

/// Frames a second: the time is counted in frames, so the app's clock stops with it.
pub const FPS: u64 = 60;
/// A card picked with the D-pad stays picked this long.
const PICK_MS: u64 = 8000;
/// START held this long gives up the Pomodoro phase.
const HOLD_FRAMES: u64 = 45;
/// The heartbeat is every 5 s: this long with no line from the computer means no link.
pub const NO_LINK_MS: u64 = 15_000;

/// Which screen the app shows while it is paired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Duty,
    Pomodoro,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Pair,
    Forget,
    Mute,
    Close,
}

pub const MENU: [MenuItem; 4] = [MenuItem::Pair, MenuItem::Forget, MenuItem::Mute, MenuItem::Close];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    pub at: usize,
    /// "Forget computers" asks first.
    pub confirm: bool,
}

pub struct App {
    pub server: Server,
    pub duty: Duty,
    pub pomodoro: Pomodoro,
    pub keys: Keys,
    pub frames: u64,
    pub page: Page,
    pub menu: Option<Menu>,
    /// The card picked with the D-pad, and when.
    pub picked: Option<(usize, u64)>,
    /// When the linked computer last sent a line; None until a computer links.
    pub heard: Option<u64>,
    pub muted: bool,
    /// QingOS's own mute switch.
    pub system_muted: bool,
    sounds: [Option<Vec<u8>>; 3],
    start_down: Option<u64>,
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

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let computers = host::data_read("computers").map(|bytes| identity::load_computers(&bytes)).unwrap_or_default();
        let muted = host::data_read("settings")
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|settings| settings["muted"].as_bool())
            .unwrap_or(false);
        let server = Server::new(identity(), computers, env!("CARGO_PKG_VERSION"), Box::new(random), 0);
        let app = Self {
            server,
            duty: Duty::default(),
            pomodoro: Pomodoro::default(),
            keys: Keys::default(),
            frames: 0,
            page: Page::Duty,
            menu: None,
            picked: None,
            heard: None,
            muted,
            system_muted: false,
            sounds: ["input_required.wav", "done.wav", "failed.wav"].map(host::asset_read),
            start_down: None,
        };
        app.answer_mdns();
        app
    }

    /// Milliseconds since the app started, by its frames.
    pub fn now(&self) -> u64 {
        self.frames * 1000 / FPS
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
                    if let Some(sound) = self.duty.apply(&event, self.now()) {
                        self.play(sound);
                    }
                }
            }
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
        };
        if let Some(wav) = &self.sounds[at] {
            tone_wav(wav);
        }
    }

    /// A short chime when a Pomodoro phase ends: the robot's voice has no line for it that fits.
    fn chime(&self, ended: Phase) {
        if self.muted || self.system_muted {
            return;
        }
        let hz = if ended == Phase::Focus { 880 } else { 660 };
        tone(Channel::Square1, hz, 180, 40);
        tone(Channel::Square2, hz * 3 / 2, 360, 30);
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
        let _ = host::data_write("settings", json!({"muted": self.muted}).to_string().as_bytes());
    }

    /// One frame: keys, time, then the caller draws.
    pub fn step(&mut self, buttons: u32) {
        self.frames += 1;
        self.keys.set(buttons);
        let now = self.now();
        let out = self.server.tick(now);
        self.carry_out(out);
        self.duty.tick(now);
        if let Some(ended) = self.pomodoro.tick(now) {
            self.page = Page::Pomodoro;
            self.chime(ended);
        }
        if self.picked.is_some_and(|(_, at)| now.saturating_sub(at) >= PICK_MS) {
            self.picked = None;
        }
        self.keys_pressed(now);
    }

    fn keys_pressed(&mut self, now: u64) {
        let k = self.keys;
        if self.menu.is_some() {
            return self.menu_keys(now);
        }
        if self.server.code().is_some() {
            // The pairing screen: B goes back when there are computers already.
            if k.pressed(key::B) {
                self.server.hide_code();
            }
            if k.pressed(key::X) && !self.server.computers().is_empty() {
                self.menu = Some(Menu { at: 0, confirm: false });
            }
            return;
        }
        if k.pressed(key::X) {
            self.menu = Some(Menu { at: 0, confirm: false });
            return;
        }
        if k.pressed(key::SELECT) {
            self.page = if self.page == Page::Duty { Page::Pomodoro } else { Page::Duty };
            self.picked = None;
        }
        // START: pressed and let go starts, pauses or goes on; held gives up.
        if k.pressed(key::START) {
            self.start_down = Some(self.frames);
        }
        if let Some(down) = self.start_down {
            if k.held(key::START) && self.frames - down == HOLD_FRAMES {
                self.pomodoro.stop();
                self.page = Page::Pomodoro;
            } else if k.released(key::START) {
                if self.frames - down < HOLD_FRAMES {
                    self.pomodoro.toggle(now);
                    self.page = Page::Pomodoro;
                }
                self.start_down = None;
            }
        }
        if self.page != Page::Duty || !self.link_alive() {
            return;
        }
        let cards = self.duty.tasks.len();
        if cards > 0 && (k.pressed(key::DOWN) || k.pressed(key::UP)) {
            let at = match (self.picked, k.pressed(key::DOWN)) {
                (None, true) => 0,
                (None, false) => cards - 1,
                (Some((at, _)), true) => (at + 1) % cards,
                (Some((at, _)), false) => (at + cards - 1) % cards,
            };
            self.picked = Some((at, now));
        }
        if k.pressed(key::B) {
            self.picked = None;
        }
        if k.pressed(key::A) {
            let task = self.picked.and_then(|(at, _)| self.duty.tasks.get(at)).and_then(|task| task.id.clone());
            match task {
                Some(task) => self.send(json!({"version": 1, "event": "focus", "task": task})),
                None => self.send(json!({"version": 1, "event": "button", "button": "K2", "action": "press"})),
            }
            self.picked = None;
        }
    }

    fn menu_keys(&mut self, now: u64) {
        let k = self.keys;
        let Some(mut menu) = self.menu else { return };
        if menu.confirm {
            if k.pressed(key::A) {
                let out = self.server.forget_all(now);
                self.carry_out(out);
                self.duty = Duty::default();
                self.heard = None;
                self.menu = None;
            } else if k.pressed(key::B) {
                self.menu = Some(Menu { confirm: false, ..menu });
            }
            return;
        }
        if k.pressed(key::B) || k.pressed(key::X) {
            self.menu = None;
            return;
        }
        if k.pressed(key::DOWN) {
            menu.at = (menu.at + 1) % MENU.len();
        }
        if k.pressed(key::UP) {
            menu.at = (menu.at + MENU.len() - 1) % MENU.len();
        }
        self.menu = Some(menu);
        if k.pressed(key::A) {
            match MENU[menu.at] {
                MenuItem::Pair => {
                    self.server.show_code(now);
                    self.menu = None;
                }
                MenuItem::Forget => self.menu = Some(Menu { confirm: true, ..menu }),
                MenuItem::Mute => {
                    self.muted = !self.muted;
                    self.save_settings();
                }
                MenuItem::Close => self.menu = None,
            }
        }
    }
}
