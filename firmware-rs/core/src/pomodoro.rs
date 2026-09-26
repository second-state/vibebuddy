//! 番茄钟：专注 25 分钟，休息 5 分钟。时长暂时固定。
//!
//! 本模块不读时钟，所有入口都由调用方传入毫秒计数，允许回绕。

use crate::TIME_SCALE;

pub const FOCUS_MS: u32 = 25 * 60 * 1000;
pub const BREAK_MS: u32 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Focus,
    Break,
}

/// 每个阶段都由用户按键开始，不自动衔接：专注结束后停在“休息待开始”，
/// 休息结束后停在“专注待开始”（也就是空闲）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Pending,
    Running,
    Paused,
}

/// 阶段结束是只消费一次的边沿：语音与模式切换都挂在它上面，
/// 同一次结束不得触发第二遍。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Nothing,
    FocusEnded,
    BreakEnded,
}

/// 当日记录：完成的专注次数与累计专注秒数。日期来自 Mac 端心跳，
/// 变了就清零；重启后由存储恢复。只记完成的专注，放弃的不算。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// 本地日期 YYYYMMDD；0 表示还没从 Mac 端听说过今天是哪天。
    pub day: u32,
    pub completed: u32,
    pub focus_s: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub run: Run,
    /// 当前阶段还剩多少毫秒；待开始时给出这一阶段的全长。
    pub remaining_ms: u32,
    /// 当前阶段的全长，画圆环时作分母。
    pub total_ms: u32,
    pub completed: u32,
    pub focus_s: u32,
}

impl View {
    /// 空闲：还没开始的专注。
    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Focus && self.run == Run::Pending
    }
}

pub struct Pomodoro {
    phase: Phase,
    run: Run,
    /// 运行中：阶段截止的毫秒计数。
    deadline_ms: u32,
    /// 暂停中：剩余毫秒。暂停时把时间冻结成一个数，恢复时再展开成截止时刻。
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

    /// 用有符号差值比较，让毫秒计数回绕时仍然正确。
    fn remaining_while_running(&self, now_ms: u32) -> u32 {
        let left = self.deadline_ms.wrapping_sub(now_ms) as i32;
        if left > 0 { left as u32 } else { 0 }
    }

    /// 短按：待开始时开始这一阶段；运行中暂停；暂停中继续。
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

    /// 长按：放弃当前阶段，回到空闲。休息待开始时按它就是跳过休息。
    /// 已完成的次数不受影响。
    pub fn stop(&mut self) {
        self.phase = Phase::Focus;
        self.run = Run::Pending;
    }

    /// 推进时钟。阶段刚结束时返回对应的转换，之后返回 Nothing。
    pub fn tick(&mut self, now_ms: u32) -> Transition {
        if self.run != Run::Running || (now_ms.wrapping_sub(self.deadline_ms) as i32) < 0 {
            return Transition::Nothing;
        }
        // 下一阶段停在待开始，等用户按键：休息什么时候开始、下一段专注什么
        // 时候开始，都是用户的决定。
        self.run = Run::Pending;
        if self.phase == Phase::Focus {
            self.tally.completed += 1;
            // 记的是完整的一段专注，不是压缩后的长度：验收固件跑 25 秒也算 25 分钟。
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

    /// 重启后从存储恢复当日记录。
    pub fn restore_tally(&mut self, tally: Tally) {
        self.tally = tally;
    }

    /// 心跳里的本地日期。与记录的日期不同就清零并记下新日期；返回 true 表示
    /// 记录变了、该存一次。0 表示不知道，忽略。
    pub fn set_day(&mut self, day: u32) -> bool {
        if day == 0 || day == self.tally.day {
            return false;
        }
        // 换日：清零。第一次听说日期时记录本来就是空的，清零也无妨；
        // 若重启后恢复的是昨天的记录，正好在这里归零。
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
        // 晚到 40 ms 的 tick 仍然只报一次结束。
        assert_eq!(pomodoro.tick(FOCUS_MS + 40), Transition::FocusEnded);
        assert_eq!(pomodoro.tick(FOCUS_MS + 80), Transition::Nothing);

        // 休息不自动开始：停在待开始，时间不走。
        let view = pomodoro.view(FOCUS_MS + 60000);
        assert_eq!(view.phase, Phase::Break);
        assert_eq!(view.run, Run::Pending);
        assert!(!view.is_idle());
        assert_eq!(view.completed, 1);
        assert_eq!(view.total_ms, BREAK_MS);
        assert_eq!(view.remaining_ms, BREAK_MS);
        assert_eq!(pomodoro.tick(FOCUS_MS + 3_600_000), Transition::Nothing);

        // 用户按键才开始休息，从按键时刻起算。
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

        // 暂停后再放弃，恢复的旧剩余时间不能泄漏到下一次专注里。
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
        // 跳过休息不抹掉已经完成的那一次专注。
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

        // 放弃的不算。
        pomodoro.stop();
        pomodoro.toggle(0);
        pomodoro.stop();
        let tally = pomodoro.tally();
        assert_eq!(tally.completed, 2);
        assert_eq!(tally.day, 20260915);

        // 换日清零，日期同样时不清。
        assert!(pomodoro.set_day(20260916));
        assert_eq!(pomodoro.tally(), Tally { day: 20260916, completed: 0, focus_s: 0 });

        // 重启后恢复昨天的记录，心跳带来今天的日期才归零。
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
