//! The firmware's main program: dispatch between the serial line protocol, keys, pomodoro,
//! leisure, voice and screen. Mirrors vibebuddy_fw.c in the C firmware. All hardware sits
//! behind [`Board`], so the whole chain runs as tests on the Mac: feed a JSON line, then see
//! what it replied, drew and played.
//!
//! Every line of serial output matches the C firmware verbatim, so the Mac's parsing needs no change.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use serde_json::{Map, Value};
use vibebuddy_protocol::{Event, ProtocolError, VERSION};

use crate::audio::{Prompt, clamp_volume, VOLUME_DEFAULT};
use crate::buttons::{ButtonEvent, Buttons, Levels};
use crate::display::{Display, MAX_STATS, MAX_TASKS, Mode, Panel, PanelKind, Scene, Screen, State, TaskInput, Tone};
use crate::leisure::{Leisure, Tier};
use crate::menu::{self, Action as MenuAction, Context as MenuContext, Menu, Row, View};
use crate::pomodoro::{Phase, Pomodoro, Run, Transition};
use crate::storage::{Flash, Settings, SettingsStore, find_partition};
use crate::text;
use crate::voice_pack::crc32;
use crate::voices::{ClipTable, Voices, VoiceError};

pub const MAX_LINE_BYTES: usize = 1024;
const LINE_BUFFER_BYTES: usize = MAX_LINE_BYTES + 2;
/// With no message for longer than this, the link to the Mac counts as lost.
const LINK_TIMEOUT_MS: u32 = 15000;
/// After task.done, return to idle on our own after this long.
const DONE_TO_IDLE_MS: u32 = 5000;
/// voice.begin waits at most this long for the line being played to finish; the longest line is under 7 seconds.
const AUDIO_DRAIN_MS: u32 = 10000;

pub const FIRMWARE_NAME: &str = "vibebuddy-fw 0.1.0";

/// Result of audio init: the codec model (ES8311 or NS4168) on success, or the step it got
/// stuck at on failure. Either is reported to the Mac as is.
pub type AudioStatus = Result<&'static str, &'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeError {
    /// The NS4168 variant has no codec, so the volume can't be adjusted.
    NotSupported,
    Failed,
}

/// Screenshot callback: gets the read-only framebuffer and a way to write to the serial port.
pub type FrameAction<'a> = dyn FnMut(&[u8], &mut dyn FnMut(&[u8])) + 'a;

/// Everything the device layer provides.
pub trait Board: Screen {
    /// Monotonic millisecond count; wraparound is allowed.
    fn now_ms(&self) -> u32;
    /// Writes to both UART0 and USB Serial/JTAG. These are only diagnostic channels and neither
    /// may hold up the main loop: if the side with no reader can't take a write, drop it.
    fn write(&mut self, bytes: &[u8]);
    fn flash(&mut self) -> &mut dyn Flash;
    /// Lends out the framebuffer (read-only) and serial output together: a screenshot reads the frame while writing it out.
    fn with_frame_and_output(&mut self, action: &mut FrameAction);

    /// Initializes the display hardware (backlight stays off).
    fn init_display(&mut self) -> bool;
    /// Initializes I2S and the codec, and turns sound on at the given volume.
    fn init_audio(&mut self, volume: u32) -> AudioStatus;
    /// Initializes the keys and returns the state of all three at that moment.
    fn init_buttons(&mut self) -> Option<Levels>;

    /// K0's level and the levels of K1 and K2 on the expander (true means pressed); None if the expander read fails.
    fn read_buttons(&mut self) -> (bool, Option<(bool, bool)>);

    /// Queues an announcement. With `clips` None, plays the built-in voice. Returns Err when the queue is full.
    fn play(&mut self, prompt: Prompt, clips: ClipTable) -> Result<(), ()>;
    /// Stops what is playing and clears the queue.
    fn stop_audio(&mut self);
    /// Still making sound: something is queued, playing, or not yet flushed to silence in DMA.
    fn audio_busy(&self) -> bool;
    fn set_volume(&mut self, level: u32) -> Result<(), VolumeError>;

    /// Boots the other app slot when the box shares its flash with one (Muse); returns false, without
    /// rebooting, on a box that has only this firmware.
    fn boot_other_app(&mut self) -> bool {
        false
    }

    /// Whether the other app slot holds an app, so the menu can offer to switch to it.
    fn has_other_app(&mut self) -> bool {
        false
    }
}

pub struct Firmware {
    pomodoro: Pomodoro,
    leisure: Leisure,
    display: Display,
    voices: Voices,
    buttons: Option<Buttons>,
    settings: Option<SettingsStore>,
    volume: u32,
    audio_ready: bool,
    build: Vec<u8>,

    line: Vec<u8>,
    discarding: bool,
    last_message_ms: u32,
    link_lost: bool,
    ready_deadline: Option<u32>,
    /// Mute: long-press K2 to toggle; not persisted. Mute it for a meeting and forget, and the
    /// device would stay silent for days; coming back with sound after a restart is safer than
    /// remembering, and the MUTE badge on screen is the reminder.
    muted: bool,
    menu: Menu,
    /// Read once at boot: the flash layout doesn't change while running.
    other_app: bool,
    /// voice.begin arrived and we are waiting for playback to stop before erasing the partition: total bytes and when the wait began.
    pending_voice_begin: Option<(u32, u32)>,

    /// The level, backlight state and hour last reported to the Mac; each is reported as a line only when it changes.
    reported_tier: Tier,
    reported_lights_out: bool,
    reported_hour: i32,
}

/// Borrows the Firmware's pomodoro and leisure director to build the context for one frame.
macro_rules! scene {
    ($self:ident, $now:expr) => {
        Scene { now_ms: $now, pomodoro: &$self.pomodoro, leisure: &$self.leisure }
    };
}

fn task_state(status: &str) -> State {
    match status {
        "input_required" => State::InputRequired,
        "done" => State::Done,
        "failed" => State::Failed,
        _ => State::Working,
    }
}

/// A subset of strtoul: skip leading whitespace and read the leading decimal digits; no digits means 0.
fn leading_number(text: &str) -> u32 {
    text.trim_start().bytes().take_while(u8::is_ascii_digit).fold(0u32, |value, digit| {
        value.wrapping_mul(10).wrapping_add((digit - b'0') as u32)
    })
}

/// Both the envelope's extra fields (BTreeMap) and task cards (JSON objects) can be looked up by key.
trait Fields {
    fn field(&self, key: &str) -> Option<&Value>;
}

impl Fields for Map<alloc::string::String, Value> {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl Fields for BTreeMap<alloc::string::String, Value> {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

fn number(fields: &impl Fields, key: &str) -> Option<f64> {
    fields.field(key).and_then(Value::as_f64)
}

fn string<'a>(fields: &'a impl Fields, key: &str) -> Option<&'a str> {
    fields.field(key).and_then(Value::as_str)
}

impl Firmware {
    pub fn new(seed: u32, now_ms: u32, build: &[u8]) -> Self {
        Self {
            pomodoro: Pomodoro::new(),
            leisure: Leisure::new(seed, now_ms),
            display: Display::new(),
            voices: Voices::new(),
            buttons: None,
            settings: None,
            volume: VOLUME_DEFAULT,
            audio_ready: false,
            build: build.to_vec(),
            line: Vec::with_capacity(LINE_BUFFER_BYTES),
            discarding: false,
            last_message_ms: now_ms,
            link_lost: false,
            ready_deadline: None,
            muted: false,
            menu: Menu::new(),
            other_app: false,
            pending_voice_begin: None,
            reported_tier: Tier::Alert,
            reported_lights_out: false,
            reported_hour: -1,
        }
    }

    fn write_literal<B: Board>(board: &mut B, text: &str) {
        board.write(text.as_bytes());
    }

    /// One `label value` line, with CR and LF in value replaced by spaces.
    fn write_value_line<B: Board>(board: &mut B, label: &str, value: &[u8]) {
        board.write(label.as_bytes());
        let mut start = 0;
        for (index, &byte) in value.iter().enumerate() {
            if byte == b'\r' || byte == b'\n' {
                board.write(&value[start..index]);
                board.write(b" ");
                start = index + 1;
            }
        }
        board.write(&value[start..]);
        board.write(b"\n");
    }

    /// Boot: initialize each part in the C firmware's order and report the same lines.
    pub fn boot<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        // Restore yesterday's record too: whether the day changed is only known once a heartbeat brings the date.
        // On a box shared with Muse, nvs is Muse's own NVS and the log lives in vb_cfg.
        let settings_region = find_partition(board.flash(), "vb_cfg").or_else(|| find_partition(board.flash(), "nvs"));
        let loaded = settings_region.map(|region| SettingsStore::open(board.flash(), region));
        match loaded {
            Some(Ok((store, found))) => {
                let tally = found.map(|settings| settings.tally).unwrap_or_default();
                if let Some(settings) = found {
                    self.volume = clamp_volume(settings.volume);
                }
                self.pomodoro.restore_tally(tally);
                self.settings = Some(store);
                let line = text!(48, "TALLY LOADED {} {}S DAY {}\n", tally.completed % 10000, tally.focus_s % 1_000_000, tally.day % 100_000_000);
                board.write(line.as_bytes());
            }
            _ => Self::write_literal(board, "TALLY LOAD ERROR\n"),
        }

        let display_ok = board.init_display() && self.display.start(board, &scene!(self, now)).is_ok();
        if display_ok {
            let build = self.build.clone();
            self.display.set_firmware_build(board, &scene!(self, now), &build);
            Self::write_value_line(board, "DISPLAY READY BUILD ", &self.build);
        } else {
            Self::write_literal(board, "DISPLAY ERROR\n");
        }

        let voices_region = find_partition(board.flash(), "voices");
        match self.voices.init(board.flash(), voices_region) {
            Ok(()) => Self::write_value_line(board, "VOICES ", self.voices.current_id().as_bytes()),
            Err(_) => Self::write_literal(board, "VOICES NO PARTITION\n"),
        }

        match board.init_audio(self.volume) {
            Ok(codec) => {
                self.audio_ready = true;
                Self::write_literal(board, "AUDIO READY\n");
                Self::write_value_line(board, "AUDIO CODEC ", codec.as_bytes());
                self.announce_volume(board);
            }
            Err(step) => Self::write_value_line(board, "AUDIO ERROR ", step.as_bytes()),
        }

        self.other_app = board.has_other_app();
        match board.init_buttons() {
            Some(levels) => {
                self.buttons = Some(Buttons::new(levels, board.now_ms()));
                Self::write_literal(board, "BUTTONS READY\n");
            }
            None => Self::write_literal(board, "BUTTONS ERROR\n"),
        }

        self.last_message_ms = board.now_ms();
        Self::write_value_line(board, "READY ", FIRMWARE_NAME.as_bytes());
    }

    /// Bytes received on the serial port; both UART and USB feed in here.
    pub fn receive<B: Board>(&mut self, board: &mut B, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                if self.discarding {
                    Self::write_literal(board, "ERROR input_too_large\n");
                } else {
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    if self.line.len() > MAX_LINE_BYTES {
                        Self::write_literal(board, "ERROR input_too_large\n");
                    } else {
                        self.last_message_ms = board.now_ms();
                        self.set_link_lost(board, false);
                        let line = core::mem::take(&mut self.line);
                        self.handle_line(board, &line);
                        self.line = line;
                    }
                }
                self.line.clear();
                self.discarding = false;
                continue;
            }
            if !self.discarding {
                if self.line.len() == LINE_BUFFER_BYTES - 1 {
                    self.discarding = true;
                } else {
                    self.line.push(byte);
                }
            }
        }
    }

    /// Called once per main loop pass: link-loss detection, delayed return to idle, keys, pomodoro, leisure, animation.
    pub fn poll<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        if now.wrapping_sub(self.last_message_ms) as i32 >= LINK_TIMEOUT_MS as i32 {
            self.set_link_lost(board, true);
        }
        if let Some(deadline) = self.ready_deadline
            && now.wrapping_sub(deadline) as i32 >= 0
        {
            self.ready_deadline = None;
            match self.display.settle_after_done(board, &scene!(self, now)) {
                Ok(State::Working) => Self::write_literal(board, "DISPLAY STATE WORKING\n"),
                Ok(State::InputRequired) => Self::write_literal(board, "DISPLAY STATE INPUT REQUIRED\n"),
                Ok(_) => Self::write_literal(board, "DISPLAY STATE READY\n"),
                Err(()) => Self::write_literal(board, "DISPLAY ERROR\n"),
            }
        }
        self.continue_voice_begin(board);
        if self.menu.expire(now) {
            Self::write_literal(board, "MENU CLOSED\n");
            self.show_menu(board);
        }
        self.tick_buttons(board);
        let transition = self.pomodoro.tick(board.now_ms());
        self.handle_pomodoro_transition(board, transition);
        self.tend_leisure(board);
        let now = board.now_ms();
        self.display.tick(board, &scene!(self, now));
    }

    fn set_link_lost<B: Board>(&mut self, board: &mut B, lost: bool) {
        self.link_lost = lost;
        let now = board.now_ms();
        self.display.set_link_lost(board, &scene!(self, now), lost);
    }

    fn set_mode<B: Board>(&mut self, board: &mut B, mode: Mode) {
        if self.display.mode() == mode {
            return;
        }
        let now = board.now_ms();
        self.display.set_mode(board, &scene!(self, now), mode);
        Self::write_value_line(board, "MODE ", mode.name().as_bytes());
    }

    fn refresh<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        self.display.refresh(board, &scene!(self, now));
    }

    fn report_pomodoro<B: Board>(board: &mut B, what: &str) {
        Self::write_value_line(board, "POMODORO ", what.as_bytes());
    }

    fn clips(&self) -> ClipTable {
        if self.pending_voice_begin.is_some() { None } else { self.voices.clips() }
    }

    /// Every voice line goes out through here; when muted it only logs a line.
    fn play_prompt<B: Board>(&mut self, board: &mut B, prompt: Prompt, label: &str) {
        if self.muted {
            Self::write_value_line(board, "AUDIO MUTED ", label.as_bytes());
            return;
        }
        if !self.audio_ready || board.play(prompt, self.clips()).is_err() {
            Self::write_literal(board, "AUDIO ERROR\n");
        } else {
            Self::write_value_line(board, "AUDIO QUEUED ", label.as_bytes());
        }
    }

    /// Volume is the device's own fact and the app's slider is just a remote: report a line after a change, and on hello too.
    fn announce_volume<B: Board>(&self, board: &mut B) {
        let line = text!(24, "VOLUME {}\n", self.volume % 1000);
        board.write(line.as_bytes());
    }

    fn save_settings<B: Board>(&mut self, board: &mut B) -> bool {
        let settings = Settings { tally: self.pomodoro.tally(), volume: self.volume };
        match self.settings.as_mut() {
            Some(store) => store.save(board.flash(), &settings).is_ok(),
            None => false,
        }
    }

    /// When today's record changes, save it once and report a line to the Mac. It happens a few times a day.
    fn save_tally<B: Board>(&mut self, board: &mut B) {
        let tally = self.pomodoro.tally();
        let line = text!(48, "POMODORO TODAY {} {}S DAY {}\n", tally.completed % 10000, tally.focus_s % 1_000_000, tally.day % 100_000_000);
        board.write(line.as_bytes());
        if !self.save_settings(board) {
            Self::write_literal(board, "TALLY SAVE ERROR\n");
        }
    }

    /// Brings the pomodoro to the front; if it is already there, just redraws.
    fn show_pomodoro<B: Board>(&mut self, board: &mut B) {
        if self.display.mode() == Mode::Pomodoro {
            self.refresh(board);
        } else {
            self.set_mode(board, Mode::Pomodoro);
        }
    }

    fn tick_buttons<B: Board>(&mut self, board: &mut B) {
        let Some(buttons) = self.buttons.as_mut() else {
            return;
        };
        let (k0, expander) = board.read_buttons();
        let events = buttons.update(k0, expander, board.now_ms());
        for event in events.into_iter().flatten() {
            self.on_button(board, event);
        }
    }

    /// Each of the three keys does one thing regardless of mode: K0 is the pomodoro key, K1
    /// switches between duty and pomodoro (long press opens the menu), and K2 asks the Mac to
    /// open the source. While the menu is open the keys are its own (menu.rs). In leisure mode any key first calls the buddy back to duty and then does
    /// its own job: leisure hides nothing that needs a look first.
    fn on_button<B: Board>(&mut self, board: &mut B, event: ButtonEvent) {
        if event == ButtonEvent::SwitchApp {
            self.switch_app(board);
            return;
        }
        let now = board.now_ms();
        if self.menu.is_open() {
            self.leisure.note_activity(now);
            let action = self.menu.key(event, self.menu_context(now), now);
            self.on_menu_action(board, action);
            return;
        }
        let mode_before = self.display.mode();
        self.leisure.note_activity(now);
        if mode_before == Mode::Leisure {
            self.leisure.tick(now);
            self.set_mode(board, Mode::Duty);
        }

        match event {
            ButtonEvent::K2Short => {
                Self::write_literal(board, "{\"version\":1,\"event\":\"button\",\"button\":\"K2\",\"action\":\"press\"}\n");
                return;
            }
            ButtonEvent::K2Long => {
                self.toggle_mute(board);
                return;
            }
            ButtonEvent::K1Short => {
                self.set_mode(board, if mode_before == Mode::Pomodoro { Mode::Duty } else { Mode::Pomodoro });
                return;
            }
            ButtonEvent::K1Long => {
                self.menu.open(self.menu_context(now), now);
                Self::write_literal(board, "MENU OPEN\n");
                self.show_menu(board);
                return;
            }
            ButtonEvent::K0Short | ButtonEvent::K0Long | ButtonEvent::SwitchApp => {}
        }

        if event == ButtonEvent::K0Long {
            self.stop_phase(board);
            return;
        }
        let before = self.pomodoro.view(now);
        let break_phase = before.phase == Phase::Break;
        self.pomodoro.toggle(now);
        if before.run == Run::Pending {
            Self::report_pomodoro(board, if break_phase { "BREAK START" } else { "FOCUS START" });
            // Starting a phase brings the pomodoro to the front: the ring starting to move is the feedback.
            self.show_pomodoro(board);
            return;
        }
        // Pause and resume don't change scenes; the badge in the top right of the buddy scene flashes along.
        Self::report_pomodoro(board, if before.run == Run::Paused { "RESUMED" } else { "PAUSED" });
        self.refresh(board);
    }

    /// Gives up the current Pomodoro phase: K0 long, or STOP FOCUS in the menu.
    fn stop_phase<B: Board>(&mut self, board: &mut B) {
        let before = self.pomodoro.view(board.now_ms());
        if before.is_idle() {
            return;
        }
        self.pomodoro.stop();
        let skipped = before.phase == Phase::Break && before.run == Run::Pending;
        Self::report_pomodoro(board, if skipped { "BREAK SKIPPED" } else { "STOPPED" });
        self.refresh(board);
    }

    /// K2 long, or MUTE in the menu.
    fn toggle_mute<B: Board>(&mut self, board: &mut B) {
        self.muted = !self.muted;
        let now = board.now_ms();
        let muted = self.muted;
        self.display.set_muted(board, &scene!(self, now), muted);
        Self::write_literal(board, if muted { "MUTE ON\n" } else { "MUTE OFF\n" });
    }

    /// Sets, saves and reports the volume: the app's slider and the menu both land here.
    fn set_volume_level<B: Board>(&mut self, board: &mut B, level: u32) {
        let level = clamp_volume(level);
        let saved = match board.set_volume(level) {
            Ok(()) => {
                self.volume = level;
                self.save_settings(board)
            }
            Err(_) => false,
        };
        if !saved {
            Self::write_literal(board, "VOLUME ERROR\n");
        }
        self.announce_volume(board);
    }

    /// K1 + K2 held, or MUSE in the menu: boots the other app; returns only if it can't.
    fn switch_app<B: Board>(&mut self, board: &mut B) {
        Self::write_literal(board, "SWITCH APP\n");
        if !board.boot_other_app() {
            Self::write_literal(board, "SWITCH APP UNAVAILABLE\n");
        }
    }

    fn menu_context(&self, now: u32) -> MenuContext {
        MenuContext { phase_active: !self.pomodoro.view(now).is_idle(), other_app: self.other_app }
    }

    fn on_menu_action<B: Board>(&mut self, board: &mut B, action: MenuAction) {
        match action {
            MenuAction::None => return,
            MenuAction::Redraw => {}
            MenuAction::Close => Self::write_literal(board, "MENU CLOSED\n"),
            MenuAction::StopPhase => {
                Self::write_literal(board, "MENU CLOSED\n");
                self.show_menu(board);
                self.stop_phase(board);
            }
            MenuAction::SwitchApp => {
                Self::write_literal(board, "MENU CLOSED\n");
                self.show_menu(board);
                self.switch_app(board);
            }
            // The preview goes through play_prompt, so it is silent when muted, like the app's.
            MenuAction::StepVolume => {
                self.set_volume_level(board, menu::next_volume(self.volume));
                self.play_prompt(board, Prompt::Done, "DONE");
            }
            MenuAction::ToggleMute => self.toggle_mute(board),
        }
        self.show_menu(board);
    }

    /// Something that needs the user takes the screen back from the menu.
    fn close_menu<B: Board>(&mut self, board: &mut B) {
        if self.menu.is_open() {
            self.menu.close();
            Self::write_literal(board, "MENU CLOSED\n");
            self.show_menu(board);
        }
    }

    /// Hands the display the panel for what the menu shows now, or none when it's closed.
    fn show_menu<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        let panel = self.menu.view().map(|view| self.menu_panel(view, now));
        self.display.set_panel(board, &scene!(self, now), panel);
    }

    fn menu_panel(&self, view: View, now: u32) -> Panel {
        let line = |label: &[u8], value: &[u8], tone| (label.to_vec(), value.to_vec(), tone);
        match view {
            View::List => {
                let context = self.menu_context(now);
                let rows = Menu::rows(context);
                let selected = self.menu.selected(context);
                let volume = text!(4, "{}", self.volume % 1000);
                let lines = rows
                    .iter()
                    .map(|row| match row {
                        Row::StopPhase if self.pomodoro.view(now).phase == Phase::Break => line(b"END BREAK", b"", Tone::Plain),
                        Row::StopPhase => line(b"STOP FOCUS", b"", Tone::Plain),
                        Row::Volume => line(b"VOLUME", volume.as_bytes(), Tone::Accent),
                        Row::Mute => line(b"MUTE", if self.muted { b"ON" } else { b"OFF" }, Tone::Accent),
                        Row::Muse => line(b"MUSE", b"SWITCH", Tone::Dim),
                        Row::Status => line(b"STATUS", b"OPEN", Tone::Dim),
                    })
                    .collect();
                let verb: &'static [u8] = match rows[selected] {
                    Row::StopPhase => b"STOP",
                    Row::Volume => b"CHANGE",
                    Row::Mute => b"TOGGLE",
                    Row::Muse => b"SWITCH",
                    Row::Status => b"OPEN",
                };
                Panel {
                    kind: PanelKind::List,
                    title: b"MENU".to_vec(),
                    lines,
                    selected: Some(selected),
                    hints: alloc::vec![(b"K1", b"NEXT"), (b"K0", verb), (b"K2", b"CLOSE")],
                }
            }
            View::Status => {
                let build = self.build.split(|&byte| byte == b' ').next().unwrap_or(&[]);
                let voice = self.voices.current_id().to_ascii_uppercase();
                let volume = text!(4, "{}", self.volume % 1000);
                let tally = self.pomodoro.tally();
                let minutes = tally.focus_s / 60;
                let today = if minutes >= 60 {
                    text!(24, "{} FOCUS {}H{:02}", tally.completed % 1000, minutes / 60 % 100, minutes % 60)
                } else {
                    text!(24, "{} FOCUS {}M", tally.completed % 1000, minutes)
                };
                Panel {
                    kind: PanelKind::Facts,
                    title: b"STATUS".to_vec(),
                    lines: alloc::vec![
                        line(b"FIRMWARE", &build[..build.len().min(13)], Tone::Plain),
                        line(b"MAC", if self.link_lost { b"NO LINK" } else { b"LINKED" }, Tone::Plain),
                        line(b"VOICE", voice.as_bytes(), Tone::Plain),
                        line(b"VOLUME", volume.as_bytes(), Tone::Plain),
                        line(b"MUTE", if self.muted { b"ON" } else { b"OFF" }, Tone::Plain),
                        line(b"TODAY", today.as_bytes(), Tone::Plain),
                    ],
                    selected: None,
                    hints: alloc::vec![(b"K0", b"BACK"), (b"K2", b"CLOSE")],
                }
            }
            View::ConfirmMuse => Panel {
                kind: PanelKind::Message,
                title: b"SWITCH TO MUSE?".to_vec(),
                lines: alloc::vec![
                    line(b"THE BOX RESTARTS INTO MUSE.", b"", Tone::Plain),
                    line(b"HOLD K1 AND K2 THERE FOR", b"", Tone::Plain),
                    line(b"3 SECONDS TO COME BACK.", b"", Tone::Plain),
                    line(b"", b"", Tone::Plain),
                    line(b"AGENT CARDS PAUSE UNTIL", b"", Tone::Dim),
                    line(b"YOU RETURN.", b"", Tone::Dim),
                ],
                selected: None,
                hints: alloc::vec![(b"K0", b"SWITCH"), (b"K2", b"CANCEL")],
            },
        }
    }

    /// A phase end is an edge consumed once: play the voice once and bring the pomodoro to the
    /// front, since this is exactly when the user should glance at it. The next phase waits to
    /// start until the user presses K0.
    fn handle_pomodoro_transition<B: Board>(&mut self, board: &mut B, transition: Transition) {
        let focus_ended = match transition {
            Transition::Nothing => return,
            Transition::FocusEnded => true,
            Transition::BreakEnded => false,
        };
        Self::report_pomodoro(board, if focus_ended { "FOCUS END" } else { "BREAK END" });
        // The alarm waits for K0, which the menu would take.
        self.close_menu(board);
        if focus_ended {
            self.save_tally(board);
        }
        if focus_ended {
            self.play_prompt(board, Prompt::FocusDone, "FOCUS_DONE");
        } else {
            self.play_prompt(board, Prompt::BreakDone, "BREAK_DONE");
        }
        let now = board.now_ms();
        self.display.pomodoro_ended(&scene!(self, now));
        self.show_pomodoro(board);
    }

    /// Boredom accumulates on state, not on messages: agents at work, a running pomodoro or a
    /// lost link are not idle. After duty has been idle long enough, go to leisure; come back
    /// the moment something happens. In pomodoro mode, a still 25:00 waiting to start is as dull
    /// as idle duty and leaves after five minutes; a pause is the user stopping there on
    /// purpose, so only after half an hour is the user assumed gone. Either way it returns to
    /// duty first and then drifts into leisure naturally.
    fn tend_leisure<B: Board>(&mut self, board: &mut B) {
        let now = board.now_ms();
        let pomodoro = self.pomodoro.view(now);
        if !self.display.agent_idle() || pomodoro.run == Run::Running || self.link_lost || self.menu.is_open() {
            self.leisure.note_activity(now);
        }

        let mode = self.display.mode();
        let changed = self.leisure.tick(now);
        let leisure = self.leisure.view(now);
        // Report the level by difference, not by "did it change this time": the wake path advances the director elsewhere first.
        if leisure.tier != self.reported_tier {
            self.reported_tier = leisure.tier;
            Self::write_value_line(board, "LEISURE ", leisure.tier.name().as_bytes());
        } else if changed && mode == Mode::Leisure {
            Self::write_value_line(board, "LEISURE SKIT ", leisure.skit.name().as_bytes());
        }
        if leisure.lights_out != self.reported_lights_out {
            self.reported_lights_out = leisure.lights_out;
            Self::write_literal(board, if leisure.lights_out { "LEISURE LIGHTS OUT\n" } else { "LEISURE LIGHTS ON\n" });
        }

        if mode == Mode::Duty && leisure.tier != Tier::Alert {
            self.set_mode(board, Mode::Leisure);
        } else if mode == Mode::Leisure && leisure.tier == Tier::Alert {
            self.set_mode(board, Mode::Duty);
        } else if mode == Mode::Pomodoro {
            let pending = pomodoro.run == Run::Pending;
            if leisure.tier == Tier::Sleepy || (pending && leisure.tier == Tier::Bored) {
                self.set_mode(board, Mode::Duty);
            }
        }
    }

    /// Today's stats come with every state event, and the idle screen rotates through them.
    ///
    /// Taking them only from `agent.idle` isn't enough: after `task.done` the device returns to
    /// idle on its own, and that is exactly when the user glances over, so the cached stats
    /// must already include the task just finished.
    fn parse_stats(&mut self, fields: &BTreeMap<alloc::string::String, Value>) {
        let Some(Value::Array(items)) = fields.get("stats") else {
            return;
        };
        let lines: Vec<&str> = items.iter().filter_map(Value::as_str).take(MAX_STATS).collect();
        let bytes: Vec<&[u8]> = lines.iter().map(|line| line.as_bytes()).collect();
        self.display.set_stats(&bytes);
        // The number in the "7 DONE" line: in leisure it decides whether the buddy is tired or bored.
        let done = lines.iter().find(|line| line.contains("DONE")).map(|line| leading_number(line)).unwrap_or(0);
        self.leisure.set_done_count(done);
    }

    fn show_event<B: Board>(&mut self, board: &mut B, fields: &BTreeMap<alloc::string::String, Value>, event: &str, title: Option<&str>) {
        let task_items: Vec<&Map<alloc::string::String, Value>> = match fields.get("tasks") {
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_object).collect(),
            _ => Vec::new(),
        };
        let mut tasks: Vec<TaskInput> = Vec::new();
        for item in task_items {
            if tasks.len() == MAX_TASKS {
                break;
            }
            let (Some(task_title), Some(status)) = (string(item, "title"), string(item, "status")) else {
                continue;
            };
            tasks.push(TaskInput {
                title: Some(task_title.as_bytes()),
                state: task_state(status),
                elapsed_s: number(item, "elapsed_s").map(|value| value as i32).unwrap_or(0),
                project: string(item, "project").map(str::as_bytes),
            });
        }

        let now = board.now_ms();
        let (state, state_label, mut prompt) = match event {
            "task.start" => (State::Working, "WORKING", None),
            "agent.idle" => (State::Idle, "READY", None),
            "agent.input_required" => (State::InputRequired, "INPUT REQUIRED", Some((Prompt::InputRequired, "INPUT_REQUIRED"))),
            "task.done" => (State::Done, "DONE", Some((Prompt::Done, "DONE"))),
            "task.error" | "agent.blocked" => (State::Failed, "FAILED", Some((Prompt::Failed, "FAILED"))),
            _ => return,
        };
        self.ready_deadline = if event == "task.done" { Some(now.wrapping_add(DONE_TO_IDLE_MS)) } else { None };

        if fields.get("suppress_audio") == Some(&Value::Bool(true)) {
            prompt = None;
        }
        match string(fields, "announcement") {
            Some("done") => prompt = Some((Prompt::Done, "DONE")),
            Some("failed") => prompt = Some((Prompt::Failed, "FAILED")),
            _ => {}
        }

        if matches!(state, State::InputRequired | State::Failed) {
            self.close_menu(board);
        }
        let shown = self.display.show_tasks(board, &scene!(self, now), state, title.map(str::as_bytes), &tasks);
        if shown.is_err() {
            Self::write_literal(board, "DISPLAY ERROR\n");
        } else {
            Self::write_value_line(board, "DISPLAY STATE ", state_label.as_bytes());
        }
        if let Some((prompt, label)) = prompt {
            self.play_prompt(board, prompt, label);
        }
    }

    fn handle_line<B: Board>(&mut self, board: &mut B, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            Self::write_literal(board, "ERROR invalid_json\n");
            return;
        };
        // title may be absent, but if present it must be a string; the protocol envelope checks the rest.
        let title_ok = match value.get("title") {
            None => true,
            Some(title) => title.is_string(),
        };
        let parsed = if value.is_object() && title_ok { serde_json::from_value::<Event>(value).ok() } else { None };
        let Some(message) = parsed else {
            Self::write_literal(board, "ERROR invalid_message\n");
            return;
        };
        match message.validate() {
            Ok(()) => {}
            Err(ProtocolError::UnsupportedVersion(_)) => {
                Self::write_literal(board, "ERROR unsupported_version\n");
                return;
            }
            Err(_) => {
                Self::write_literal(board, "ERROR invalid_message\n");
                return;
            }
        }
        debug_assert_eq!(message.version, VERSION);
        let event = message.event.as_str();
        let fields = &message.extra;
        let now = board.now_ms();

        match event {
            // The heartbeat only proves the link is alive; it isn't shown or echoed, since a diagnostic
            // line every 5 seconds would drown the log. It also carries the Mac's build stamp: the
            // device may restart at any time, and a one-off handshake would be lost.
            "device.heartbeat" => {
                if let Some(build) = string(fields, "build") {
                    self.display.set_daemon_build(board, &scene!(self, now), build.as_bytes());
                }
                // The local hour also comes with the heartbeat: the device has no clock, so day and night are whatever the Mac says.
                if let Some(hour) = number(fields, "hour").map(|hour| hour as i32)
                    && hour != self.reported_hour
                {
                    self.reported_hour = hour;
                    self.leisure.set_hour(hour);
                    let line = text!(24, "CLOCK HOUR {}\n", hour % 100);
                    board.write(line.as_bytes());
                }
                // The local date also comes with the heartbeat: the pomodoro's daily record resets on it.
                if let Some(day) = number(fields, "day")
                    && day > 0.0
                    && self.pomodoro.set_day(day as u32)
                {
                    self.save_tally(board);
                    self.refresh(board);
                }
            }
            // A screenshot is a debugging action: it isn't agent activity and doesn't wake leisure.
            "device.screenshot" => {
                let display = &self.display;
                board.with_frame_and_output(&mut |frame, output| {
                    display.dump(frame, &mut |line| {
                        output(line);
                        output(b"\n");
                    });
                });
            }
            // The Mac asks when it first connects: mode, firmware build and voice are only reported
            // at boot or on change, and the daemon restarts more often than the device, so without
            // asking it would never know.
            "device.hello" => self.announce_state(board),
            // Blink-to-identify and voice pack writes are the app operating the device itself, so they aren't agent activity either.
            "device.identify" => {
                self.display.identify(now);
                Self::write_literal(board, "IDENTIFY\n");
            }
            // Volume: the app's slider lands here, goes to the codec and is saved; without a
            // level it is just a query. The preview goes through play_prompt, so it is silent when
            // muted, same rule as every other announcement.
            "device.volume" => {
                match number(fields, "level") {
                    Some(level) => self.set_volume_level(board, if level < 0.0 { 0 } else { level as u32 }),
                    None => self.announce_volume(board),
                }
                // The app's slider can move while the menu shows the volume.
                self.show_menu(board);
                if fields.get("preview") == Some(&Value::Bool(true)) {
                    self.play_prompt(board, Prompt::Done, "DONE");
                }
            }
            _ if event.starts_with("voice.") => self.handle_voice_event(board, fields, event),
            // Link self-test: send the received string's length and CRC back to the Mac to check for corrupted serial bytes.
            "device.echo" => {
                if let Some(data) = string(fields, "data") {
                    let reply = text!(96, "{{\"version\":1,\"event\":\"echo\",\"length\":{},\"crc\":{}}}\n", data.len(), crc32(0, data.as_bytes()));
                    board.write(reply.as_bytes());
                    // Echo the line back verbatim; the Mac compares byte by byte to see what the port actually received.
                    Self::write_value_line(board, "ECHO ", data.as_bytes());
                }
            }
            _ => {
                // As soon as an agent does something, the buddy comes straight back to duty.
                self.leisure.note_activity(now);
                if self.display.mode() == Mode::Leisure {
                    self.leisure.tick(now);
                    self.set_mode(board, Mode::Duty);
                }
                self.parse_stats(fields);
                Self::write_value_line(board, "EVENT ", event.as_bytes());
                if let Some(title) = &message.title {
                    Self::write_value_line(board, "TITLE ", title.as_bytes());
                }
                self.show_event(board, fields, event, message.title.as_deref());
            }
        }
    }

    /// Reports the device's whole static state: firmware build, mode, voice and volume. Both
    /// boot and hello go through here.
    fn announce_state<B: Board>(&self, board: &mut B) {
        Self::write_value_line(board, "DISPLAY READY BUILD ", &self.build);
        Self::write_value_line(board, "MODE ", self.display.mode().name().as_bytes());
        Self::write_value_line(board, "VOICES ", self.voices.current_id().as_bytes());
        self.announce_volume(board);
    }

    /// Voice pack write acknowledgements are all JSON lines: the Mac does stop-and-wait flow control by sequence number, which diagnostic lines can't support.
    fn voice_reply<B: Board>(board: &mut B, event: &str, seq: i64, detail: &str) {
        let line = match event {
            "voice.written" => text!(160, "{{\"version\":1,\"event\":\"voice.written\",\"voice\":\"{}\"}}\n", detail),
            "voice.error" => text!(160, "{{\"version\":1,\"event\":\"voice.error\",\"seq\":{},\"message\":\"{}\"}}\n", seq, detail),
            _ => text!(160, "{{\"version\":1,\"event\":\"{}\",\"seq\":{}}}\n", event, seq),
        };
        board.write(line.as_bytes());
    }

    fn handle_voice_event<B: Board>(&mut self, board: &mut B, fields: &BTreeMap<alloc::string::String, Value>, event: &str) {
        match event {
            "voice.begin" => {
                let Some(size) = number(fields, "size") else {
                    Self::voice_reply(board, "voice.error", -1, VoiceError::InvalidArg.name());
                    return;
                };
                // A missing partition or a wrong size is an immediate error, without interrupting the line being played.
                if let Err(error) = self.voices.check_begin(size as u32) {
                    Self::voice_reply(board, "voice.error", -1, error.name());
                    return;
                }
                // The line being played may be reading this very partition: stop playback and let DMA
                // flush to silence first, then erase. Erasing takes several seconds with the main loop
                // stalled, and DMA replays whatever is in its buffer over and over, so it must be
                // silence first. The wait continues in poll.
                board.stop_audio();
                self.pending_voice_begin = Some((size as u32, board.now_ms()));
                self.continue_voice_begin(board);
            }
            "voice.chunk" => {
                let seq = number(fields, "seq");
                let data = string(fields, "data");
                let crc = number(fields, "crc");
                let (Some(seq), Some(data), Some(crc)) = (seq, data, crc) else {
                    Self::voice_reply(board, "voice.error", -1, "invalid chunk");
                    self.voices.abort(board.flash());
                    return;
                };
                match self.voices.chunk(board.flash(), seq as u32, data.as_bytes(), crc as u32) {
                    Ok(()) => Self::voice_reply(board, "voice.ack", seq as i64, ""),
                    Err(error) => {
                        Self::voice_reply(board, "voice.error", seq as i32 as i64, error.name());
                        self.voices.abort(board.flash());
                    }
                }
            }
            "voice.end" => {
                if let Err(error) = self.voices.end(board.flash()) {
                    Self::voice_reply(board, "voice.error", -1, error.name());
                    self.voices.abort(board.flash());
                    return;
                }
                let id = alloc::string::String::from(self.voices.current_id());
                Self::voice_reply(board, "voice.written", -1, &id);
                Self::write_value_line(board, "VOICES ", id.as_bytes());
                // Say a line in the new voice when done: over the bridge a write takes several minutes,
                // and the user may not be watching the app for the preview button; the box speaking up
                // is the most direct "done" (on 2026-09-22 a colleague finished a write and thought
                // there was no sound).
                self.play_prompt(board, Prompt::Done, "DONE");
            }
            _ => Self::voice_reply(board, "voice.error", -1, "unknown voice event"),
        }
    }

    /// The second half of voice.begin: once playback has stopped (or we've waited long enough), erase the partition and reply voice.ready.
    fn continue_voice_begin<B: Board>(&mut self, board: &mut B) {
        let Some((size, since)) = self.pending_voice_begin else {
            return;
        };
        if board.audio_busy() && (board.now_ms().wrapping_sub(since) as i32) < AUDIO_DRAIN_MS as i32 {
            return;
        }
        self.pending_voice_begin = None;
        match self.voices.begin(board.flash(), size) {
            Ok(()) => Self::voice_reply(board, "voice.ready", -1, ""),
            Err(error) => Self::voice_reply(board, "voice.error", -1, error.name()),
        }
    }
}
