//! Pomodoro: 25 minutes of focus, 5 minutes of break. Durations are fixed for now.
//!
//! This module reads no clock; every entry point takes a millisecond count from the caller, and wraparound is allowed.

use crate::TIME_SCALE;

pub const FOCUS_MS: u32 = 25 * 60 * 1000;
pub const BREAK_MS: u32 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    Break,
}

/// Every phase starts with a key press, never automatically: after focus ends it waits
/// at "break ready", and after the break ends at "focus ready" (that is, idle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Pending,
    Running,
    Paused,
}

/// A phase end is an edge consumed once: voice and mode switches hang off it, so the
/// same end must not fire twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Nothing,
    FocusEnded,
    BreakEnded,
}

/// Today's record: completed focus sessions and total focus seconds. The date comes from
/// the Mac's heartbeat and a change resets it; it is restored from storage after a
/// restart. Only completed focus sessions count, abandoned ones don't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Local date as YYYYMMDD; 0 means the Mac hasn't told us today's date yet.
    pub day: u32,
    pub completed: u32,
    pub focus_s: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub run: Run,
    /// Milliseconds left in the current phase; the phase's full length while waiting to start.
    pub remaining_ms: u32,
    /// Full length of the current phase, the denominator when drawing the ring.
    pub total_ms: u32,
    pub completed: u32,
    pub focus_s: u32,
}

impl View {
    /// Idle: a focus session not yet started.
    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Focus && self.run == Run::Pending
    }
}

pub struct Pomodoro {
    phase: Phase,
    run: Run,
    /// Running: the millisecond count at which the phase ends.
    deadline_ms: u32,
    /// Paused: remaining milliseconds. Pausing freezes time into one number; resuming turns it back into a deadline.
    remaining_ms: u32,
    tally: Tally,
}

fn phase_length(phase: Phase) -> u32 {
    let full = match phase {
        Phase::Break => BREAK_MS,
        Phase::Focus => FOCUS_MS,
    };
    full / TIME_SCALE
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self::new()
    }
}

impl Pomodoro {
    pub const fn new() -> Self {
        Self { phase: Phase::Focus, run: Run::Pending, deadline_ms: 0, remaining_ms: 0, tally: Tally { day: 0, completed: 0, focus_s: 0 } }
    }

    /// Compares by signed difference so it stays correct when the millisecond count wraps.
    fn remaining_while_running(&self, now_ms: u32) -> u32 {
        let left = self.deadline_ms.wrapping_sub(now_ms) as i32;
        if left > 0 { left as u32 } else { 0 }
    }

    /// Short press: start the phase when waiting; pause when running; resume when paused.
    pub fn toggle(&mut self, now_ms: u32) {
        match self.run {
            Run::Pending => {
                self.run = Run::Running;
                self.deadline_ms = now_ms.wrapping_add(phase_length(self.phase));
            }
            Run::Paused => {
                self.run = Run::Running;
                self.deadline_ms = now_ms.wrapping_add(self.remaining_ms);
            }
            Run::Running => {
                self.run = Run::Paused;
                self.remaining_ms = self.remaining_while_running(now_ms);
            }
        }
    }

    /// Long press: abandon the current phase and return to idle. Pressed while the break is
    /// waiting to start, it skips the break. Completed counts are unaffected.
    pub fn stop(&mut self) {
        self.phase = Phase::Focus;
        self.run = Run::Pending;
    }

    /// Advances the clock. Returns the matching transition right when a phase ends, Nothing afterwards.
    pub fn tick(&mut self, now_ms: u32) -> Transition {
        if self.run != Run::Running || (now_ms.wrapping_sub(self.deadline_ms) as i32) < 0 {
            return Transition::Nothing;
        }
        // The next phase waits to start until the user presses a key: when the break
        // starts and when the next focus starts are both the user's call.
        self.run = Run::Pending;
        if self.phase == Phase::Focus {
            self.tally.completed += 1;
            // Record a full focus session, not the compressed length: 25 seconds on acceptance firmware still counts as 25 minutes.
            self.tally.focus_s += FOCUS_MS / 1000;
            self.phase = Phase::Break;
            return Transition::FocusEnded;
        }
        self.phase = Phase::Focus;
        Transition::BreakEnded
    }

    pub fn view(&self, now_ms: u32) -> View {
        let total_ms = phase_length(self.phase);
        let remaining_ms = match self.run {
            Run::Pending => total_ms,
            Run::Paused => self.remaining_ms,
            Run::Running => self.remaining_while_running(now_ms),
        };
        View {
            phase: self.phase,
            run: self.run,
            remaining_ms,
            total_ms,
            completed: self.tally.completed,
            focus_s: self.tally.focus_s,
        }
    }

    /// Restores today's record from storage after a restart.
    pub fn restore_tally(&mut self, tally: Tally) {
        self.tally = tally;
    }

    /// The local date from a heartbeat. If it differs from the recorded date, reset and store
    /// the new date; returns true when the record changed and should be saved. 0 means
    /// unknown and is ignored.
    pub fn set_day(&mut self, day: u32) -> bool {
        if day == 0 || day == self.tally.day {
            return false;
        }
        // New day: reset. The first time we hear a date the record is empty anyway, so
        // resetting is harmless; if a restart restored yesterday's record, it resets here.
        self.tally = Tally { day, completed: 0, focus_s: 0 };
        true
    }

    pub fn tally(&self) -> Tally {
        self.tally
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_shows_a_full_focus() {
        let mut pomodoro = Pomodoro::new();
        let view = pomodoro.view(0);
        assert!(view.is_idle());
        assert_eq!(view.remaining_ms, FOCUS_MS);
        assert_eq!(view.total_ms, FOCUS_MS);
        assert_eq!(view.completed, 0);
        assert_eq!(pomodoro.tick(1000), Transition::Nothing);
    }

    #[test]
    fn toggle_starts_pauses_and_resumes() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.toggle(1000);
        let view = pomodoro.view(2000);
        assert_eq!(view.phase, Phase::Focus);
        assert_eq!(view.run, Run::Running);
        assert_eq!(view.remaining_ms, FOCUS_MS - 1000);

        pomodoro.toggle(6000);
        let view = pomodoro.view(60000);
        assert_eq!(view.run, Run::Paused);
        assert_eq!(view.remaining_ms, FOCUS_MS - 5000);
        assert_eq!(pomodoro.tick(60000), Transition::Nothing);

        pomodoro.toggle(60000);
        let view = pomodoro.view(61000);
        assert_eq!(view.run, Run::Running);
        assert_eq!(view.remaining_ms, FOCUS_MS - 6000);
    }

    #[test]
    fn focus_end_waits_for_the_user_to_start_the_break() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.toggle(0);
        assert_eq!(pomodoro.tick(FOCUS_MS - 1), Transition::Nothing);
        // A tick 40 ms late still reports the end only once.
        assert_eq!(pomodoro.tick(FOCUS_MS + 40), Transition::FocusEnded);
        assert_eq!(pomodoro.tick(FOCUS_MS + 80), Transition::Nothing);

        // The break doesn't start on its own: it waits to start and the time doesn't run.
        let view = pomodoro.view(FOCUS_MS + 60000);
        assert_eq!(view.phase, Phase::Break);
        assert_eq!(view.run, Run::Pending);
        assert!(!view.is_idle());
        assert_eq!(view.completed, 1);
        assert_eq!(view.total_ms, BREAK_MS);
        assert_eq!(view.remaining_ms, BREAK_MS);
        assert_eq!(pomodoro.tick(FOCUS_MS + 3_600_000), Transition::Nothing);

        // The break starts only on a key press, counted from the moment of the press.
        let break_start = FOCUS_MS + 90000;
        pomodoro.toggle(break_start);
        let view = pomodoro.view(break_start + 1000);
        assert_eq!(view.run, Run::Running);
        assert_eq!(view.remaining_ms, BREAK_MS - 1000);

        let break_end = break_start + BREAK_MS;
        assert_eq!(pomodoro.tick(break_end - 1), Transition::Nothing);
        assert_eq!(pomodoro.tick(break_end), Transition::BreakEnded);
        assert_eq!(pomodoro.tick(break_end + 1), Transition::Nothing);
        let view = pomodoro.view(break_end + 1);
        assert!(view.is_idle());
        assert_eq!(view.remaining_ms, FOCUS_MS);
        assert_eq!(view.completed, 1);
    }

    #[test]
    fn stop_abandons_without_counting() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.toggle(0);
        pomodoro.stop();
        let view = pomodoro.view(FOCUS_MS + 1);
        assert!(view.is_idle());
        assert_eq!(view.completed, 0);
        assert_eq!(pomodoro.tick(FOCUS_MS + 1), Transition::Nothing);

        // Abandoning after a pause: the old remaining time must not leak into the next focus.
        pomodoro.toggle(0);
        pomodoro.toggle(1000);
        pomodoro.stop();
        pomodoro.toggle(5000);
        let view = pomodoro.view(5000);
        assert_eq!(view.phase, Phase::Focus);
        assert_eq!(view.run, Run::Running);
        assert_eq!(view.remaining_ms, FOCUS_MS);
    }

    #[test]
    fn stop_skips_a_pending_break() {
        let mut pomodoro = Pomodoro::new();
        pomodoro.toggle(0);
        assert_eq!(pomodoro.tick(FOCUS_MS), Transition::FocusEnded);
        pomodoro.stop();
        let view = pomodoro.view(FOCUS_MS + 1);
        assert!(view.is_idle());
        // Skipping the break doesn't erase the focus session already completed.
        assert_eq!(view.completed, 1);
    }

    #[test]
    fn tally_counts_completed_focus_and_resets_by_day() {
        let mut pomodoro = Pomodoro::new();
        assert!(pomodoro.set_day(20260915));
        assert!(!pomodoro.set_day(20260915));
        assert!(!pomodoro.set_day(0));

        pomodoro.toggle(0);
        pomodoro.tick(FOCUS_MS);
        pomodoro.toggle(FOCUS_MS);
        pomodoro.tick(FOCUS_MS + BREAK_MS);
        pomodoro.toggle(FOCUS_MS + BREAK_MS);
        pomodoro.tick(2 * FOCUS_MS + BREAK_MS);
        let view = pomodoro.view(2 * FOCUS_MS + BREAK_MS);
        assert_eq!(view.completed, 2);
        assert_eq!(view.focus_s, 2 * FOCUS_MS / 1000);

        // Abandoned ones don't count.
        pomodoro.stop();
        pomodoro.toggle(0);
        pomodoro.stop();
        let tally = pomodoro.tally();
        assert_eq!(tally.completed, 2);
        assert_eq!(tally.day, 20260915);

        // A new day resets; the same date doesn't.
        assert!(pomodoro.set_day(20260916));
        assert_eq!(pomodoro.tally(), Tally { day: 20260916, completed: 0, focus_s: 0 });

        // After a restart yesterday's record is restored, and it resets only when a heartbeat brings today's date.
        let mut pomodoro = Pomodoro::new();
        pomodoro.restore_tally(Tally { day: 20260916, completed: 3, focus_s: 4500 });
        let view = pomodoro.view(0);
        assert_eq!(view.completed, 3);
        assert_eq!(view.focus_s, 4500);
        assert!(!pomodoro.set_day(20260916));
        assert_eq!(pomodoro.view(0).completed, 3);
        assert!(pomodoro.set_day(20260917));
        assert_eq!(pomodoro.view(0).completed, 0);
    }

    #[test]
    fn millisecond_counter_may_wrap() {
        let mut pomodoro = Pomodoro::new();
        let start = u32::MAX - 1000;
        pomodoro.toggle(start);
        let wrapped = start.wrapping_add(5000);
        assert_eq!(wrapped, 3999);
        assert_eq!(pomodoro.view(wrapped).remaining_ms, FOCUS_MS - 5000);
        assert_eq!(pomodoro.tick(wrapped), Transition::Nothing);
        assert_eq!(pomodoro.tick(start.wrapping_add(FOCUS_MS)), Transition::FocusEnded);
    }
}
