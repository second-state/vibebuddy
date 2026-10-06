//! The device menu (docs/device-menu.md): the box's own settings behind a long press on K1,
//! stepped through with short presses only. K1 moves to the next row, K0 acts on it and K2 closes;
//! values cycle on K0, so nothing inside needs a long press. This module only decides what a key
//! means here; the firmware carries out the action and the display draws the panel.

use alloc::vec::Vec;

use crate::buttons::ButtonEvent;
use crate::text::Text;

/// Left alone this long, the menu closes on its own.
pub const IDLE_CLOSE_MS: u32 = 30_000;
/// K0 on VOLUME steps through these and wraps; six presses cover the app's 20 to 100.
pub const VOLUME_STEPS: [u32; 6] = [20, 35, 50, 65, 80, 100];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// Give up the running or paused Pomodoro phase, as K0 long does.
    StopPhase,
    Volume,
    Mute,
    Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    List,
    Status,
}

/// What decides which rows show, read afresh on every key.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// A Pomodoro phase is running or paused.
    pub phase_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The key meant nothing here.
    None,
    /// The menu changed what it shows.
    Redraw,
    Close,
    /// This closes the menu first.
    StopPhase,
    /// These keep it open.
    StepVolume,
    ToggleMute,
}

pub struct Menu {
    view: Option<View>,
    /// Kept as a row rather than an index: rows come and go (STOP FOCUS once the phase is given up).
    selected: Row,
    last_key_ms: u32,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    pub fn new() -> Self {
        Self { view: None, selected: Row::Volume, last_key_ms: 0 }
    }

    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    pub fn view(&self) -> Option<View> {
        self.view
    }

    pub fn rows(context: Context) -> Vec<Row> {
        let mut rows = Vec::with_capacity(4);
        if context.phase_active {
            rows.push(Row::StopPhase);
        }
        rows.extend([Row::Volume, Row::Mute, Row::Status]);
        rows
    }

    /// The selected row's index among `rows(context)`; the first row if it has gone.
    pub fn selected(&self, context: Context) -> usize {
        Self::rows(context).iter().position(|&row| row == self.selected).unwrap_or(0)
    }

    /// Always opens on the first row.
    pub fn open(&mut self, context: Context, now_ms: u32) {
        self.view = Some(View::List);
        self.selected = Self::rows(context)[0];
        self.last_key_ms = now_ms;
    }

    pub fn close(&mut self) {
        self.view = None;
    }

    pub fn key(&mut self, event: ButtonEvent, context: Context, now_ms: u32) -> Action {
        let Some(view) = self.view else {
            return Action::None;
        };
        self.last_key_ms = now_ms;
        match (view, event) {
            (_, ButtonEvent::K2Short) => {
                self.close();
                Action::Close
            }
            (View::List, ButtonEvent::K1Short) => {
                let rows = Self::rows(context);
                self.selected = rows[(self.selected(context) + 1) % rows.len()];
                Action::Redraw
            }
            (View::List, ButtonEvent::K0Short) => match Self::rows(context)[self.selected(context)] {
                Row::StopPhase => {
                    self.close();
                    Action::StopPhase
                }
                Row::Volume => Action::StepVolume,
                Row::Mute => Action::ToggleMute,
                Row::Status => self.show(View::Status),
            },
            (View::Status, ButtonEvent::K0Short) => self.show(View::List),
            _ => Action::None,
        }
    }

    fn show(&mut self, view: View) -> Action {
        self.view = Some(view);
        Action::Redraw
    }

    /// Closes the menu once it has been left alone for IDLE_CLOSE_MS; true if it just did.
    pub fn expire(&mut self, now_ms: u32) -> bool {
        if self.is_open() && now_ms.wrapping_sub(self.last_key_ms) as i32 >= IDLE_CLOSE_MS as i32 {
            self.close();
            return true;
        }
        false
    }
}

/// The next volume step above `current`, wrapping to the lowest. A level set from the app that
/// isn't a step moves to the next step up.
pub fn next_volume(current: u32) -> u32 {
    VOLUME_STEPS.iter().copied().find(|&step| step > current).unwrap_or(VOLUME_STEPS[0])
}

/// A build stamp as the status view shows it: the version and the build time, short enough for
/// one row. `v0.3.2-11-g0adc18b-dirty 2026-10-06 16:34` becomes `0.3.2-11!` and `10-06 16:34`:
/// the hash and year go, the commits past the tag stay (without them a dev build reads as the
/// release), and `!` marks uncommitted changes since the font has no `+` or `*`. The daemon's stamp
/// leads with the app version (`0.3.1 v0.3.1-12-ga4a667c ...`); the git description after it wins.
pub fn short_build(build: &[u8]) -> (Text<16>, Text<12>) {
    let mut words: Vec<&[u8]> = build.split(|&byte| byte == b' ').filter(|word| !word.is_empty()).collect();
    let mut time = Text::new();
    if let [.., date, clock] = words[..]
        && date.len() == 10
        && date[4] == b'-'
        && clock.len() == 5
        && clock[2] == b':'
    {
        time.push_bytes(&date[5..]);
        time.push_bytes(b" ");
        time.push_bytes(clock);
        words.truncate(words.len() - 2);
    }
    let mut version = Text::new();
    let Some(&describe) = words.last() else {
        version.push_bytes(b"?");
        return (version, time);
    };
    let (describe, dirty) = match describe.strip_suffix(b"-dirty") {
        Some(clean) => (clean, true),
        None => (describe, false),
    };
    // tag-N-gHASH: keep the tag and N.
    let mut parts = describe.rsplitn(3, |&byte| byte == b'-');
    let describe = match (parts.next(), parts.next(), parts.next()) {
        (Some(hash), Some(count), Some(tag))
            if hash.first() == Some(&b'g') && !count.is_empty() && count.iter().all(u8::is_ascii_digit) =>
        {
            &describe[..tag.len() + 1 + count.len()]
        }
        _ => describe,
    };
    let describe = match describe {
        [b'v' | b'V', digit, ..] if digit.is_ascii_digit() => &describe[1..],
        _ => describe,
    };
    version.push_bytes(describe);
    if dirty {
        version.push_bytes(b"!");
    }
    (version, time)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::string::{String, ToString};

    fn short(build: &str) -> (String, String) {
        let (version, time) = short_build(build.as_bytes());
        (String::from_utf8(version.as_bytes().to_vec()).unwrap(), String::from_utf8(time.as_bytes().to_vec()).unwrap())
    }

    #[test]
    fn short_build_keeps_the_version_and_the_time() {
        let pair = |version: &str, time: &str| (version.to_string(), time.to_string());
        assert_eq!(short("v0.3.2-11-g0adc18b 2026-10-06 16:34"), pair("0.3.2-11", "10-06 16:34"));
        assert_eq!(short("0.3.1 v0.3.1-12-ga4a667c 2026-10-06 16:26"), pair("0.3.1-12", "10-06 16:26"));
        assert_eq!(short("v0.3.2-11-g0adc18b-dirty 2026-10-06 16:34"), pair("0.3.2-11!", "10-06 16:34"));
        assert_eq!(short("v0.3.2 2026-10-06 16:34"), pair("0.3.2", "10-06 16:34"));
        assert_eq!(short("abc1234-dirty 2026-09-16 13:11"), pair("abc1234!", "09-16 13:11"));
        assert_eq!(short("v0.2.0-rc1-3-gabc1234"), pair("0.2.0-rc1-3", ""));
        assert_eq!(short("abc1234"), pair("abc1234", ""));
        assert_eq!(short(""), pair("?", ""));
    }

    const PLAIN: Context = Context { phase_active: false };
    const FULL: Context = Context { phase_active: true };

    #[test]
    fn rows_follow_the_context() {
        assert_eq!(Menu::rows(PLAIN), [Row::Volume, Row::Mute, Row::Status]);
        assert_eq!(Menu::rows(FULL), [Row::StopPhase, Row::Volume, Row::Mute, Row::Status]);
    }

    #[test]
    fn k1_steps_down_and_wraps_and_k2_closes() {
        let mut menu = Menu::new();
        assert_eq!(menu.key(ButtonEvent::K1Short, PLAIN, 0), Action::None, "closed: keys are not the menu's");
        menu.open(PLAIN, 0);
        assert_eq!(menu.selected(PLAIN), 0);
        assert_eq!(menu.key(ButtonEvent::K1Short, PLAIN, 10), Action::Redraw);
        assert_eq!(menu.key(ButtonEvent::K1Short, PLAIN, 20), Action::Redraw);
        assert_eq!(menu.selected(PLAIN), 2);
        menu.key(ButtonEvent::K1Short, PLAIN, 30);
        assert_eq!(menu.selected(PLAIN), 0);
        assert_eq!(menu.key(ButtonEvent::K2Short, PLAIN, 40), Action::Close);
        assert!(!menu.is_open());
    }

    #[test]
    fn k0_acts_on_the_selected_row() {
        let mut menu = Menu::new();
        menu.open(PLAIN, 0);
        assert_eq!(menu.key(ButtonEvent::K0Short, PLAIN, 0), Action::StepVolume);
        assert!(menu.is_open());
        menu.key(ButtonEvent::K1Short, PLAIN, 0);
        assert_eq!(menu.key(ButtonEvent::K0Short, PLAIN, 0), Action::ToggleMute);
        menu.key(ButtonEvent::K1Short, PLAIN, 0);
        assert_eq!(menu.key(ButtonEvent::K0Short, PLAIN, 0), Action::Redraw);
        assert_eq!(menu.view(), Some(View::Status));
        assert_eq!(menu.key(ButtonEvent::K1Short, PLAIN, 0), Action::None, "the status view has no rows");
        assert_eq!(menu.key(ButtonEvent::K0Short, PLAIN, 0), Action::Redraw);
        assert_eq!(menu.view(), Some(View::List));
        assert_eq!(menu.selected(PLAIN), 2, "back on STATUS");
    }

    #[test]
    fn stopping_the_phase_closes_and_the_row_goes() {
        let mut menu = Menu::new();
        menu.open(FULL, 0);
        assert_eq!(menu.key(ButtonEvent::K0Short, FULL, 0), Action::StopPhase);
        assert!(!menu.is_open());
        let after = Context { phase_active: false };
        menu.open(after, 0);
        assert_eq!(Menu::rows(after)[menu.selected(after)], Row::Volume);
    }

    #[test]
    fn long_presses_mean_nothing_inside() {
        let mut menu = Menu::new();
        menu.open(PLAIN, 0);
        for event in [ButtonEvent::K0Long, ButtonEvent::K1Long, ButtonEvent::K2Long] {
            assert_eq!(menu.key(event, PLAIN, 0), Action::None);
        }
        assert!(menu.is_open());
    }

    #[test]
    fn it_closes_after_thirty_quiet_seconds() {
        let mut menu = Menu::new();
        menu.open(PLAIN, 1000);
        menu.key(ButtonEvent::K1Short, PLAIN, 5000);
        assert!(!menu.expire(5000 + IDLE_CLOSE_MS - 1));
        assert!(menu.expire(5000 + IDLE_CLOSE_MS));
        assert!(!menu.is_open());
        assert!(!menu.expire(5000 + 2 * IDLE_CLOSE_MS), "only once");
    }

    #[test]
    fn volume_steps_up_and_wraps() {
        assert_eq!(next_volume(65), 80);
        assert_eq!(next_volume(70), 80, "a level from the app moves to the next step");
        assert_eq!(next_volume(100), 20);
        assert_eq!(next_volume(20), 35);
    }
}
