//! Pomodoro, as on the box (`firmware-rs/core/src/pomodoro.rs`): 25 minutes of focus, 5 of break. Each phase starts
//! with a press of START, never by itself. It runs in the app only; the computer never hears of it.

pub const FOCUS_MS: u64 = 25 * 60 * 1000;
pub const BREAK_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    Break,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Pending,
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug)]
pub struct Pomodoro {
    pub phase: Phase,
    pub run: Run,
    deadline: u64,
    paused_left: u64,
    /// Focus sessions finished since the app started.
    pub completed: u32,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self { phase: Phase::Focus, run: Run::Pending, deadline: 0, paused_left: 0, completed: 0 }
    }
}

impl Pomodoro {
    pub fn length(&self) -> u64 {
        match self.phase {
            Phase::Focus => FOCUS_MS,
            Phase::Break => BREAK_MS,
        }
    }

    /// START: start the phase when it waits, pause it when it runs, go on when it is paused.
    pub fn toggle(&mut self, now: u64) {
        match self.run {
            Run::Pending => (self.run, self.deadline) = (Run::Running, now + self.length()),
            Run::Paused => (self.run, self.deadline) = (Run::Running, now + self.paused_left),
            Run::Running => (self.run, self.paused_left) = (Run::Paused, self.left(now)),
        }
    }

    /// START held: give up the phase and wait to focus again. A waiting break is skipped.
    pub fn stop(&mut self) {
        (self.phase, self.run) = (Phase::Focus, Run::Pending);
    }

    /// Milliseconds left in this phase; its whole length while it waits.
    pub fn left(&self, now: u64) -> u64 {
        match self.run {
            Run::Pending => self.length(),
            Run::Paused => self.paused_left,
            Run::Running => self.deadline.saturating_sub(now),
        }
    }

    /// Returns the phase that ended, once, when it ends.
    pub fn tick(&mut self, now: u64) -> Option<Phase> {
        if self.run != Run::Running || now < self.deadline {
            return None;
        }
        let ended = self.phase;
        self.run = Run::Pending;
        self.phase = match ended {
            Phase::Focus => {
                self.completed += 1;
                Phase::Break
            }
            Phase::Break => Phase::Focus,
        };
        Some(ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_then_a_break_each_started_by_start() {
        let mut pomodoro = Pomodoro::default();
        pomodoro.toggle(0);
        assert_eq!(pomodoro.left(60_000), FOCUS_MS - 60_000);
        pomodoro.toggle(60_000);
        assert_eq!(pomodoro.run, Run::Paused);
        assert_eq!(pomodoro.left(500_000), FOCUS_MS - 60_000, "a paused clock doesn't move");
        pomodoro.toggle(500_000);
        assert_eq!(pomodoro.tick(500_000 + FOCUS_MS - 60_000), Some(Phase::Focus));
        assert_eq!(pomodoro.tick(500_000 + FOCUS_MS), None, "an end is told once");
        assert_eq!((pomodoro.phase, pomodoro.run, pomodoro.completed), (Phase::Break, Run::Pending, 1));
        pomodoro.toggle(0);
        assert_eq!(pomodoro.tick(BREAK_MS), Some(Phase::Break));
        assert_eq!((pomodoro.phase, pomodoro.run), (Phase::Focus, Run::Pending));
    }

    #[test]
    fn giving_up_goes_back_to_focus_and_keeps_the_count() {
        let mut pomodoro = Pomodoro { completed: 2, ..Pomodoro::default() };
        pomodoro.toggle(0);
        pomodoro.stop();
        assert_eq!((pomodoro.phase, pomodoro.run, pomodoro.completed), (Phase::Focus, Run::Pending, 2));
        assert_eq!(pomodoro.tick(FOCUS_MS), None);
    }
}
