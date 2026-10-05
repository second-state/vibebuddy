//! Each of the three keys does one thing, regardless of mode. Short presses fire on
//! release; holding past the threshold fires a long press, and the release then no
//! longer counts as a short press.
//!
//! On the board (ATK-DNESP32S3-BOX V1.1) K0 is the BOOT key, wired straight to GPIO0; K1 and
//! K2 are P0.4 and P0.3 on the XL9555 expander. All three are active low. Reading the pins is
//! the device layer's job; this module only handles debouncing and short vs. long presses.
//!
//! K1 and K2 held together are a chord, not two presses: neither fires its own short or long
//! press, and holding both for SWITCH_HOLD_MS boots the other app on a box that has one (Muse,
//! sharing the flash).

/// After a flip takes effect, no second flip is accepted for this long.
const DEBOUNCE_MS: u32 = 40;
const LONG_PRESS_MS: u32 = 1000;
const SWITCH_HOLD_MS: u32 = 3000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonEvent {
    /// K0: pomodoro start / pause / resume.
    K0Short,
    /// K0 long press: abandon the current phase.
    K0Long,
    /// K1: switch between duty and pomodoro.
    K1Short,
    /// K1 long press: send the buddy off to leisure right now.
    K1Long,
    /// K2: open the current source, reported to the Mac.
    K2Short,
    /// K2 long press: mute toggle.
    K2Long,
    /// K1 and K2 held together: boot the other app.
    SwitchApp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    None,
    Short,
    Long,
}

#[derive(Clone, Copy)]
struct Button {
    pressed: bool,
    changed_at: u32,
    pressed_at: u32,
    /// The long press already fired, so the release no longer counts as a short press.
    long_fired: bool,
}

impl Button {
    fn new(pressed: bool, now: u32) -> Self {
        Self { pressed, changed_at: now, pressed_at: now, long_fired: false }
    }

    /// A flip takes effect immediately; only further flips within DEBOUNCE_MS are ignored.
    /// Mechanical bounce lasts a few milliseconds while the main loop samples every 20 ms;
    /// requiring two matching samples filters out hardly any bounce but drops a quick tap
    /// entirely: in the first on-device acceptance, half the K1 presses did nothing.
    ///
    /// A short press counts on release: only then do we know it wasn't the start of a long press.
    fn update(&mut self, pressed: bool, now: u32) -> Press {
        if pressed != self.pressed {
            if (now.wrapping_sub(self.changed_at) as i32) < DEBOUNCE_MS as i32 {
                return Press::None;
            }
            self.pressed = pressed;
            self.changed_at = now;
            if pressed {
                self.pressed_at = now;
                self.long_fired = false;
                return Press::None;
            }
            return if self.long_fired { Press::None } else { Press::Short };
        }
        if pressed && !self.long_fired && now.wrapping_sub(self.pressed_at) as i32 >= LONG_PRESS_MS as i32 {
            self.long_fired = true;
            return Press::Long;
        }
        Press::None
    }
}

pub struct Buttons {
    k0: Button,
    k1: Button,
    k2: Button,
    /// When K1 and K2 were both down, while they still are.
    chord_since: Option<u32>,
    chord_fired: bool,
}

/// The pressed state of each of the three keys in one sample (true means pressed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Levels {
    pub k0: bool,
    pub k1: bool,
    pub k2: bool,
}

impl Buttons {
    pub fn new(levels: Levels, now: u32) -> Self {
        Self {
            k0: Button::new(levels.k0, now),
            k1: Button::new(levels.k1, now),
            k2: Button::new(levels.k2, now),
            chord_since: None,
            chord_fired: false,
        }
    }

    /// K0 and the expander are read separately: when the expander read fails only K0 is updated, as in the C firmware.
    pub fn update(&mut self, k0: bool, expander: Option<(bool, bool)>, now: u32) -> [Option<ButtonEvent>; 3] {
        fn emit(press: Press, short: ButtonEvent, long: ButtonEvent) -> Option<ButtonEvent> {
            match press {
                Press::None => None,
                Press::Short => Some(short),
                Press::Long => Some(long),
            }
        }
        let mut events = [None; 3];
        events[0] = emit(self.k0.update(k0, now), ButtonEvent::K0Short, ButtonEvent::K0Long);
        if let Some((k1, k2)) = expander {
            events[1] = emit(self.k1.update(k1, now), ButtonEvent::K1Short, ButtonEvent::K1Long);
            events[2] = emit(self.k2.update(k2, now), ButtonEvent::K2Short, ButtonEvent::K2Long);
            if self.k1.pressed && self.k2.pressed {
                // Marking both as long-fired silences their own long press and the release.
                self.k1.long_fired = true;
                self.k2.long_fired = true;
                events[1] = None;
                events[2] = None;
                let since = *self.chord_since.get_or_insert(now);
                if !self.chord_fired && now.wrapping_sub(since) as i32 >= SWITCH_HOLD_MS as i32 {
                    self.chord_fired = true;
                    events[1] = Some(ButtonEvent::SwitchApp);
                }
            } else {
                self.chord_since = None;
                self.chord_fired = false;
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k0(buttons: &mut Buttons, pressed: bool, now: u32) -> Option<ButtonEvent> {
        buttons.update(pressed, Some((false, false)), now)[0]
    }

    #[test]
    fn a_short_tap_fires_on_release() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        assert_eq!(k0(&mut buttons, true, 100), None);
        assert_eq!(k0(&mut buttons, false, 160), Some(ButtonEvent::K0Short));
    }

    #[test]
    fn bounces_inside_the_window_are_ignored() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        assert_eq!(k0(&mut buttons, true, 100), None);
        assert_eq!(k0(&mut buttons, false, 120), None);
        assert_eq!(k0(&mut buttons, true, 130), None);
        assert_eq!(k0(&mut buttons, false, 200), Some(ButtonEvent::K0Short));
    }

    #[test]
    fn holding_fires_long_once_and_release_is_silent() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        k0(&mut buttons, true, 100);
        assert_eq!(k0(&mut buttons, true, 1099), None);
        assert_eq!(k0(&mut buttons, true, 1100), Some(ButtonEvent::K0Long));
        assert_eq!(k0(&mut buttons, true, 2500), None);
        assert_eq!(k0(&mut buttons, false, 2600), None);
    }

    #[test]
    fn expander_buttons_map_to_k1_and_k2() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        buttons.update(false, Some((true, false)), 100);
        assert_eq!(buttons.update(false, Some((false, false)), 200), [None, Some(ButtonEvent::K1Short), None]);
        buttons.update(false, Some((false, true)), 300);
        assert_eq!(buttons.update(false, Some((false, true)), 1300), [None, None, Some(ButtonEvent::K2Long)]);
    }

    #[test]
    fn holding_k1_and_k2_switches_apps_once_and_nothing_else() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        assert_eq!(buttons.update(false, Some((true, false)), 100), [None; 3]);
        assert_eq!(buttons.update(false, Some((true, true)), 300), [None; 3]);
        // Past the long-press threshold of either key: still nothing.
        assert_eq!(buttons.update(false, Some((true, true)), 1500), [None; 3]);
        assert_eq!(buttons.update(false, Some((true, true)), 3299), [None; 3]);
        assert_eq!(buttons.update(false, Some((true, true)), 3300), [None, Some(ButtonEvent::SwitchApp), None]);
        assert_eq!(buttons.update(false, Some((true, true)), 6000), [None; 3]);
        // Letting go fires no short presses.
        assert_eq!(buttons.update(false, Some((false, true)), 6100), [None; 3]);
        assert_eq!(buttons.update(false, Some((false, false)), 6200), [None; 3]);
    }

    #[test]
    fn a_chord_let_go_early_does_nothing() {
        let mut buttons = Buttons::new(Levels::default(), 0);
        buttons.update(false, Some((true, true)), 100);
        assert_eq!(buttons.update(false, Some((false, false)), 2000), [None; 3]);
        // The keys work on their own again afterwards.
        buttons.update(false, Some((true, false)), 2100);
        assert_eq!(buttons.update(false, Some((false, false)), 2200), [None, Some(ButtonEvent::K1Short), None]);
    }
}
