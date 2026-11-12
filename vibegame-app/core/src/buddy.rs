//! The box's screens on the VibeGame: the firmware's own Display, Pomodoro and Leisure (`firmware-rs/core`), fed the
//! lines the computer sends the way the box's main program feeds them (`firmware.rs`: `handle_line`, `show_event`,
//! `poll`, `tend_leisure`, `on_button`). So the VibeGame draws what the box draws, pixel for pixel, in either set of
//! screens. Only the keys differ: the VibeGame's hints name its own (`KEYS`).

use std::vec::Vec;

use serde_json::{Map, Value};
use vibebuddy_firmware_core::agent_icon::AgentIcon;
use vibebuddy_firmware_core::audio::Occasion;
use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::{Display, KeyNames, LinkPath, MAX_STATS, MAX_TASKS, Mode, Panel, Scene, Screen, Screens, State, Target, TaskInput};
use vibebuddy_firmware_core::leisure::{Leisure, Tier};
use vibebuddy_firmware_core::pomodoro::{Pomodoro, Run, Tally, Transition, View};
use vibebuddy_protocol::Event;

/// The VibeGame's keys where the box's screens name K0, K1 and K2.
pub const KEYS: KeyNames = KeyNames { k0: "START", hold_k0: "HOLD START", k1: "SELECT", k2: "A" };
/// After `task.done` the screen settles by itself, as the box's does.
pub const DONE_TO_IDLE_MS: u32 = 5000;
/// A card picked with the D-pad stays picked this long.
pub const PICK_MS: u32 = 8000;
/// The outline of a picked card is the box's tap outline, which lasts 150 ms: it is drawn again this often.
const PICK_REDRAW_MS: u32 = 100;

/// What the VibeGame plays: the robot's three lines and the Pomodoro's chime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    InputRequired,
    Done,
    Failed,
    FocusEnded,
    BreakEnded,
}

/// What a line from the computer or a frame of time brought.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct News {
    pub sound: Option<Sound>,
    /// Needs input, failed or a Pomodoro phase that ended takes the screen: the box closes its menu for it.
    pub takes_screen: bool,
    /// Today's Pomodoro record changed: keep it.
    pub tally: bool,
}

/// The box's frame buffer, 320 × 240 RGB565 with the high byte first, and its backlight.
struct Lcd {
    bytes: Vec<u8>,
    backlight: bool,
}

impl Screen for Lcd {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    fn present(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn set_backlight(&mut self, on: bool) -> Result<(), ()> {
        self.backlight = on;
        Ok(())
    }
}

pub struct Buddy {
    display: Display,
    pomodoro: Pomodoro,
    leisure: Leisure,
    lcd: Lcd,
    link_lost: bool,
    ready_deadline: Option<u32>,
    hour: i32,
    /// A box is linked to the computer, so it speaks and the VibeGame doesn't.
    quiet: bool,
    /// The card picked with the D-pad, until when, and when its outline is drawn next.
    picked: Option<(usize, u32, u32)>,
}

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

fn string<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    fields.get(key).and_then(Value::as_str)
}

/// The cards of a state event, as `Firmware::show_event` reads them.
fn tasks(fields: &Map<String, Value>) -> Vec<TaskInput<'_>> {
    let Some(Value::Array(items)) = fields.get("tasks") else { return Vec::new() };
    items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|item| {
            let (title, status) = (string(item, "title")?, string(item, "status")?);
            Some(TaskInput {
                title: Some(title.as_bytes()),
                state: task_state(status),
                elapsed_s: item.get("elapsed_s").and_then(Value::as_f64).map_or(0, |value| value as i32),
                project: string(item, "project").map(str::as_bytes),
                // Only a short id of letters and digits goes back out in a JSON line as is.
                id: string(item, "id").filter(|id| id.len() <= 16 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())).map(str::as_bytes),
                icon: string(item, "agent").and_then(AgentIcon::for_agent),
            })
        })
        .take(MAX_TASKS)
        .collect()
}

/// The VibeGame has the built-in lines only: an occasion plays the line of the ordinary occasion it is a reading of.
fn line_for(occasion: Occasion) -> Option<Sound> {
    match occasion.fallback().unwrap_or(occasion) {
        Occasion::InputRequired => Some(Sound::InputRequired),
        Occasion::Done => Some(Sound::Done),
        Occasion::Failed => Some(Sound::Failed),
        _ => None,
    }
}

/// The leading number of a stats line, as the firmware reads "7 DONE".
fn leading_number(text: &str) -> u32 {
    text.trim_start().bytes().take_while(u8::is_ascii_digit).fold(0u32, |value, digit| value.wrapping_mul(10).wrapping_add(u32::from(digit - b'0')))
}

impl Buddy {
    /// `seed` picks the leisure skits.
    pub fn new(screens: Screens, muted: bool, tally: Option<Tally>, seed: u32, now: u32) -> Self {
        let mut buddy = Self {
            display: Display::new(),
            pomodoro: Pomodoro::new(),
            leisure: Leisure::new(seed, now),
            lcd: Lcd { bytes: vec![0; FRAME_BYTES], backlight: false },
            link_lost: false,
            ready_deadline: None,
            hour: -1,
            quiet: false,
            picked: None,
        };
        if let Some(tally) = tally {
            buddy.pomodoro.restore_tally(tally);
        }
        buddy.display.set_keys(KEYS);
        let _ = buddy.display.start(&mut buddy.lcd, &scene!(buddy, now));
        buddy.display.set_screens(&mut buddy.lcd, &scene!(buddy, now), screens);
        buddy.display.set_muted(&mut buddy.lcd, &scene!(buddy, now), muted);
        buddy
    }

    /// The box's screen as it is now, or None while its backlight is off (asleep long at night, in leisure).
    pub fn frame(&self) -> Option<&[u8]> {
        self.lcd.backlight.then_some(&self.lcd.bytes[..])
    }

    pub fn screens(&self) -> Screens {
        self.display.screens()
    }

    pub fn mode(&self) -> Mode {
        self.display.mode()
    }

    pub fn pomodoro(&self, now: u32) -> View {
        self.pomodoro.view(now)
    }

    pub fn tally(&self) -> Tally {
        self.pomodoro.tally()
    }

    pub fn set_screens(&mut self, screens: Screens, now: u32) {
        self.display.set_screens(&mut self.lcd, &scene!(self, now), screens);
    }

    pub fn set_muted(&mut self, muted: bool, now: u32) {
        self.display.set_muted(&mut self.lcd, &scene!(self, now), muted);
    }

    /// The menu over the screen, drawn as the box draws its own; None takes it away.
    pub fn set_panel(&mut self, panel: Option<Panel>, now: u32) {
        self.display.set_panel(&mut self.lcd, &scene!(self, now), panel);
    }

    /// One line from the linked computer.
    pub fn line(&mut self, value: &Value, now: u32) -> News {
        let mut news = News::default();
        if let Some(quiet) = value.get("quiet").and_then(Value::as_bool) {
            self.quiet = quiet;
        }
        if value.get("title").is_some_and(|title| !title.is_string()) {
            return news;
        }
        let Some(message) = serde_json::from_value::<Event>(value.clone()).ok().filter(|message| message.validate().is_ok()) else { return news };
        let fields: Map<String, Value> = message.extra.into_iter().collect();
        let event = message.event.as_str();
        match event {
            "device.heartbeat" => {
                if let Some(hour) = fields.get("hour").and_then(Value::as_f64).map(|hour| hour as i32)
                    && hour != self.hour
                {
                    self.hour = hour;
                    self.leisure.set_hour(hour);
                }
                if let Some(day) = fields.get("day").and_then(Value::as_f64)
                    && day > 0.0
                    && self.pomodoro.set_day(day as u32)
                {
                    news.tally = true;
                    self.display.refresh(&mut self.lcd, &scene!(self, now));
                }
            }
            // The greetings have no built-in line, and the other lines look after a box or carry voice; neither is
            // agent activity.
            "buddy.say" => {}
            _ if ["device.", "voice.", "xfer.", "link.", "vibegame."].iter().any(|prefix| event.starts_with(prefix)) => {}
            _ => {
                // As soon as an agent does something, the buddy comes straight back to duty.
                self.leisure.note_activity(now);
                if self.display.mode() == Mode::Leisure {
                    self.leisure.tick(now);
                    self.display.set_mode(&mut self.lcd, &scene!(self, now), Mode::Duty);
                }
                self.stats(&fields);
                self.show_event(&fields, event, message.title.as_deref(), now, &mut news);
            }
        }
        news
    }

    fn stats(&mut self, fields: &Map<String, Value>) {
        let Some(Value::Array(items)) = fields.get("stats") else { return };
        let lines: Vec<&str> = items.iter().filter_map(Value::as_str).take(MAX_STATS).collect();
        let bytes: Vec<&[u8]> = lines.iter().map(|line| line.as_bytes()).collect();
        self.display.set_stats(&bytes);
        let done = lines.iter().find(|line| line.contains("DONE")).map_or(0, |line| leading_number(line));
        self.leisure.set_done_count(done);
    }

    fn show_event(&mut self, fields: &Map<String, Value>, event: &str, title: Option<&str>, now: u32, news: &mut News) {
        let (state, mut occasion) = match event {
            "task.start" => (State::Working, None),
            "agent.idle" => (State::Idle, None),
            "agent.input_required" => (State::InputRequired, Some(Occasion::InputRequired)),
            "task.done" => (State::Done, Some(Occasion::Done)),
            "task.error" | "agent.blocked" => (State::Failed, Some(Occasion::Failed)),
            _ => return,
        };
        self.ready_deadline = (event == "task.done").then(|| now.wrapping_add(DONE_TO_IDLE_MS));
        if fields.get("suppress_audio") == Some(&Value::Bool(true)) {
            occasion = None;
        }
        match string(fields, "announcement") {
            Some("done") => occasion = Some(Occasion::Done),
            Some("failed") => occasion = Some(Occasion::Failed),
            _ => {}
        }
        let title = title.map(str::as_bytes);
        news.takes_screen = matches!(state, State::InputRequired | State::Failed) && !self.display.shows(state, title);
        if news.takes_screen {
            self.picked = None;
        }
        let total = fields.get("tasks_total").and_then(Value::as_f64).map_or(0, |total| total as usize);
        let _ = self.display.show_tasks(&mut self.lcd, &scene!(self, now), state, title, &tasks(fields), total);
        news.sound = if self.quiet { None } else { occasion.and_then(line_for) };
    }

    /// Once a frame. `link_alive`: the computer said something in the last 15 s. `menu_open`: the VibeGame's own
    /// screens are up, which keeps the buddy from wandering off, as the box's menu does.
    pub fn tick(&mut self, now: u32, link_alive: bool, menu_open: bool) -> News {
        let mut news = News::default();
        if self.link_lost == link_alive {
            self.link_lost = !link_alive;
            self.display.set_link_lost(&mut self.lcd, &scene!(self, now), self.link_lost);
        }
        // The VibeGame is linked over the local network: the box's icon for it, as long as the computer talks.
        let path = link_alive.then_some(LinkPath::Lan);
        self.display.set_link(&mut self.lcd, &scene!(self, now), path, false);
        if let Some(deadline) = self.ready_deadline
            && now.wrapping_sub(deadline) as i32 >= 0
        {
            self.ready_deadline = None;
            let _ = self.display.settle_after_done(&mut self.lcd, &scene!(self, now));
        }
        if let Some((card, until, redraw)) = self.picked {
            if now.wrapping_sub(until) as i32 >= 0 || self.display.mode() != Mode::Duty || self.display.panel().is_some() {
                self.picked = None;
            } else if now.wrapping_sub(redraw) as i32 >= 0 {
                self.highlight(card, until, now);
            }
        }
        let transition = self.pomodoro.tick(now);
        if transition != Transition::Nothing {
            news.takes_screen = true;
            news.tally = transition == Transition::FocusEnded;
            news.sound = Some(if transition == Transition::FocusEnded { Sound::FocusEnded } else { Sound::BreakEnded });
            self.display.set_panel(&mut self.lcd, &scene!(self, now), None);
            self.display.pomodoro_ended(&scene!(self, now));
            self.show_pomodoro(now);
        }
        self.tend_leisure(now, menu_open);
        self.display.tick(&mut self.lcd, &scene!(self, now));
        news
    }

    /// `Firmware::tend_leisure`: boredom accumulates on state, not on messages.
    fn tend_leisure(&mut self, now: u32, menu_open: bool) {
        let pomodoro = self.pomodoro.view(now);
        if !self.display.agent_idle() || pomodoro.run == Run::Running || self.link_lost || menu_open {
            self.leisure.note_activity(now);
        }
        let mode = self.display.mode();
        self.leisure.tick(now);
        let tier = self.leisure.view(now).tier;
        let next = match mode {
            Mode::Duty if tier != Tier::Alert => Mode::Leisure,
            Mode::Leisure if tier == Tier::Alert => Mode::Duty,
            Mode::Pomodoro if tier == Tier::Sleepy || (pomodoro.run == Run::Pending && tier == Tier::Bored) => Mode::Duty,
            _ => mode,
        };
        if next != mode {
            self.display.set_mode(&mut self.lcd, &scene!(self, now), next);
        }
    }

    fn show_pomodoro(&mut self, now: u32) {
        if self.display.mode() == Mode::Pomodoro {
            self.display.refresh(&mut self.lcd, &scene!(self, now));
        } else {
            self.display.set_mode(&mut self.lcd, &scene!(self, now), Mode::Pomodoro);
        }
    }

    /// Any key, before it does its own job: the buddy notices, and comes back from leisure.
    pub fn key(&mut self, now: u32) {
        self.leisure.note_activity(now);
        if self.display.mode() == Mode::Leisure {
            self.leisure.tick(now);
            self.display.set_mode(&mut self.lcd, &scene!(self, now), Mode::Duty);
        }
    }

    /// START, as the box's K0: starts a phase (and shows the Pomodoro), or pauses and goes on.
    pub fn start(&mut self, now: u32) {
        let before = self.pomodoro.view(now);
        self.pomodoro.toggle(now);
        if before.run == Run::Pending {
            self.show_pomodoro(now);
        } else {
            self.display.refresh(&mut self.lcd, &scene!(self, now));
        }
    }

    /// START held, as the box's K0 held: gives up the phase.
    pub fn stop(&mut self, now: u32) {
        if self.pomodoro.view(now).is_idle() {
            return;
        }
        self.pomodoro.stop();
        self.display.refresh(&mut self.lcd, &scene!(self, now));
    }

    /// SELECT, as the box's K1: on duty or the Pomodoro.
    pub fn switch(&mut self, now: u32) {
        let mode = if self.display.mode() == Mode::Pomodoro { Mode::Duty } else { Mode::Pomodoro };
        self.picked = None;
        self.display.set_mode(&mut self.lcd, &scene!(self, now), mode);
    }

    /// The cards up on the screen now that a `focus` can name, by where a tap would land.
    fn cards_shown(&self) -> Vec<usize> {
        let mut cards = Vec::new();
        for y in (0..HEIGHT).step_by(4) {
            for x in (0..WIDTH).step_by(4) {
                if let Some(Target::Card(card)) = self.display.target_at(x, y)
                    && !cards.contains(&card)
                    && self.display.card_id(card).is_some()
                {
                    cards.push(card);
                }
            }
        }
        cards.sort_unstable();
        cards
    }

    fn highlight(&mut self, card: usize, until: u32, now: u32) {
        self.picked = Some((card, until, now.wrapping_add(PICK_REDRAW_MS)));
        self.display.highlight(&mut self.lcd, &scene!(self, now), Target::Card(card));
    }

    /// Up or down on duty: picks the next card, turning the page past the last one shown.
    pub fn pick(&mut self, forward: bool, now: u32) {
        if self.display.mode() != Mode::Duty || self.display.panel().is_some() {
            return;
        }
        let mut cards = self.cards_shown();
        let current = self.picked.map(|(card, _, _)| card);
        let next = match current {
            None => if forward { cards.first() } else { cards.last() }.copied(),
            Some(card) if forward => cards.iter().copied().find(|&other| other > card),
            Some(card) => cards.iter().rev().copied().find(|&other| other < card),
        };
        let next = match next {
            Some(card) => Some(card),
            None => {
                if self.display.turn_page(&mut self.lcd, &scene!(self, now), forward).is_some() {
                    cards = self.cards_shown();
                }
                if forward { cards.first() } else { cards.last() }.copied()
            }
        };
        if let Some(card) = next {
            self.highlight(card, now.wrapping_add(PICK_MS), now);
        }
    }

    /// The id of the picked card, for `focus`.
    pub fn picked(&self) -> Option<String> {
        let (card, _, _) = self.picked?;
        self.display.card_id(card).map(|id| String::from_utf8_lossy(id).into_owned())
    }

    pub fn unpick(&mut self) {
        self.picked = None;
    }

    /// For setting the screen beside the firmware's previews (`tests/box_screens.rs`): what a preview sets directly
    /// that the VibeGame reaches only with time (the animation's frame, a skit) or never (the box's keys, its cable).
    #[doc(hidden)]
    pub fn preview(&mut self, now: u32, leisure: impl FnOnce(&mut Leisure), display: impl FnOnce(&mut Display, &mut dyn Screen, &Scene)) {
        leisure(&mut self.leisure);
        let scene = Scene { now_ms: now, pomodoro: &self.pomodoro, leisure: &self.leisure };
        display(&mut self.display, &mut self.lcd, &scene);
    }
}

#[cfg(test)]
mod tests;
