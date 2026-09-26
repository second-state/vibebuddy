//! 休闲模式的导演：管无聊度、抽剧目、决定什么时候转暗和关背光。
//! 不读时钟、不碰硬件，所有入口都由调用方传入毫秒计数，允许回绕。

use crate::TIME_SCALE;

/// 空闲多久算无聊、多久算困倦、夜里困倦多久后关背光。
pub const BORED_AFTER_MS: u32 = 5 * 60 * 1000 / TIME_SCALE;
pub const SLEEPY_AFTER_MS: u32 = 30 * 60 * 1000 / TIME_SCALE;
pub const LIGHTS_OUT_AFTER_MS: u32 = 90 * 60 * 1000 / TIME_SCALE;
/// 剧目动画的帧长：8 fps。
pub const FRAME_MS: u32 = 125;

/// 进入一个档位后多久开第一场；之后的间隔按档位随机。
const FIRST_SKIT_DELAY_MS: u32 = 3000;
const BORED_GAP_MIN_MS: u32 = 20000 / TIME_SCALE;
const BORED_GAP_MAX_MS: u32 = 40000 / TIME_SCALE;
const SLEEPY_GAP_MIN_MS: u32 = 120_000 / TIME_SCALE;
const SLEEPY_GAP_MAX_MS: u32 = 300_000 / TIME_SCALE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// 待命：现有的呼吸、眨眼、轮播战绩。
    Alert,
    /// 无聊：隔一会儿演一段小剧目。
    Bored,
    /// 困倦：以睡觉为主，画面转暗。
    Sleepy,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Alert => "ALERT",
            Tier::Bored => "BORED",
            Tier::Sleepy => "SLEEPY",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skit {
    /// 剧目之间的普通空闲。
    None,
    Patrol,
    Ball,
    Read,
    Stars,
    Hide,
    Startle,
    Dream,
    /// 困倦期的底色：睡觉。
    Sleep,
}

const SKIT_COUNT: usize = 9;
const ALL_SKITS: [Skit; SKIT_COUNT] =
    [Skit::None, Skit::Patrol, Skit::Ball, Skit::Read, Skit::Stars, Skit::Hide, Skit::Startle, Skit::Dream, Skit::Sleep];

impl Skit {
    pub fn name(self) -> &'static str {
        match self {
            Skit::None => "NONE",
            Skit::Patrol => "PATROL",
            Skit::Ball => "BALL",
            Skit::Read => "READ",
            Skit::Stars => "STARS",
            Skit::Hide => "HIDE",
            Skit::Startle => "STARTLE",
            Skit::Dream => "DREAM",
            Skit::Sleep => "SLEEP",
        }
    }

    /// 各剧目时长；0 表示一直演到档位变化。
    fn length_ms(self) -> u32 {
        match self {
            Skit::None | Skit::Sleep => 0,
            Skit::Patrol | Skit::Ball | Skit::Dream => 12000,
            Skit::Read | Skit::Stars => 15000,
            Skit::Hide | Skit::Startle => 10000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub tier: Tier,
    pub skit: Skit,
    /// 当前剧目已经演了几帧。
    pub skit_frame: u32,
    /// 困倦：画面转暗。
    pub dim: bool,
    /// 夜里睡久了：关背光。
    pub lights_out: bool,
}

pub struct Leisure {
    rng_state: u32,
    idle_since_ms: u32,
    hour: i32,
    done_count: u32,
    tier: Tier,
    skit: Skit,
    last_skit: Skit,
    skit_started_ms: u32,
    next_skit_at_ms: u32,
}

fn base_skit(tier: Tier) -> Skit {
    if tier == Tier::Sleepy { Skit::Sleep } else { Skit::None }
}

impl Leisure {
    pub fn new(seed: u32, now_ms: u32) -> Self {
        let mut leisure = Self {
            rng_state: if seed == 0 { 0x9e37_79b9 } else { seed },
            idle_since_ms: now_ms,
            hour: -1,
            done_count: 0,
            tier: Tier::Alert,
            skit: Skit::None,
            last_skit: Skit::None,
            skit_started_ms: now_ms,
            next_skit_at_ms: now_ms,
        };
        leisure.enter_tier(Tier::Alert, now_ms);
        leisure
    }

    fn rng_next(&mut self) -> u32 {
        // xorshift32：够随机，也够小。
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    fn rng_between(&mut self, low: u32, high: u32) -> u32 {
        low + self.rng_next() % (high - low + 1)
    }

    /// 夜里：23 点到早上 7 点。不知道几点就当白天，宁可亮着也不要在下午关灯。
    fn is_night(&self) -> bool {
        self.hour >= 23 || (self.hour >= 0 && self.hour < 7)
    }

    fn tier_for(&self, now_ms: u32) -> Tier {
        let idle = now_ms.wrapping_sub(self.idle_since_ms);
        if idle >= SLEEPY_AFTER_MS {
            Tier::Sleepy
        } else if idle >= BORED_AFTER_MS {
            Tier::Bored
        } else {
            Tier::Alert
        }
    }

    fn gap_for(&mut self, tier: Tier) -> u32 {
        if tier == Tier::Sleepy {
            self.rng_between(SLEEPY_GAP_MIN_MS, SLEEPY_GAP_MAX_MS)
        } else {
            self.rng_between(BORED_GAP_MIN_MS, BORED_GAP_MAX_MS)
        }
    }

    /// 剧目权重。夜里多睡少玩；今天一件没做就无聊地踢球，做得多就累得梦多。
    fn skit_weights(&self, tier: Tier) -> [u32; SKIT_COUNT] {
        let mut weights = [0u32; SKIT_COUNT];
        let mut set = |skit: Skit, weight: u32| weights[skit as usize] = weight;
        if tier == Tier::Sleepy {
            set(Skit::Dream, if self.done_count >= 5 { 4 } else { 3 });
            set(Skit::Startle, if self.done_count >= 5 { 0 } else { 2 });
            return weights;
        }
        let table: [(Skit, u32, u32); 7] = [
            // 剧目，夜里的权重，白天的权重
            (Skit::Patrol, 1, 3),
            (Skit::Ball, 1, 3),
            (Skit::Read, 2, 3),
            (Skit::Stars, 4, 1),
            (Skit::Hide, 1, 3),
            (Skit::Startle, 2, 1),
            (Skit::Dream, 3, 1),
        ];
        let night = self.is_night();
        for (skit, at_night, by_day) in table {
            set(skit, if night { at_night } else { by_day });
        }
        if self.done_count == 0 {
            weights[Skit::Ball as usize] += 4;
        } else if self.done_count >= 5 {
            weights[Skit::Dream as usize] += 2;
            weights[Skit::Startle as usize] += 1;
        }
        weights
    }

    fn pick_skit(&mut self, tier: Tier) -> Skit {
        let mut weights = self.skit_weights(tier);
        // 不连着演同一出；只剩一出可选时才允许重复。
        let total: u32 = weights.iter().sum();
        let without_last = total - weights[self.last_skit as usize];
        let total = if without_last > 0 {
            weights[self.last_skit as usize] = 0;
            without_last
        } else {
            total
        };
        if total == 0 {
            return base_skit(tier);
        }
        let mut roll = self.rng_next() % total;
        for skit in ALL_SKITS {
            let weight = weights[skit as usize];
            if roll < weight {
                return skit;
            }
            roll -= weight;
        }
        base_skit(tier)
    }

    fn enter_tier(&mut self, tier: Tier, now_ms: u32) {
        self.tier = tier;
        self.skit = base_skit(tier);
        self.skit_started_ms = now_ms;
        self.next_skit_at_ms = now_ms.wrapping_add(FIRST_SKIT_DELAY_MS);
    }

    /// 任何活动都把无聊度清零：Agent 有动静、按键、番茄钟在走。
    pub fn note_activity(&mut self, now_ms: u32) {
        self.idle_since_ms = now_ms;
    }

    /// K1 长按：现在就去玩。
    pub fn force_bored(&mut self, now_ms: u32) {
        self.idle_since_ms = now_ms.wrapping_sub(BORED_AFTER_MS);
    }

    /// 本地小时数，来自 Mac 端心跳；-1 表示不知道。
    pub fn set_hour(&mut self, hour: i32) {
        self.hour = hour;
    }

    /// 当日完成的专注或任务数，决定它是累了还是无聊。
    pub fn set_done_count(&mut self, done: u32) {
        self.done_count = done;
    }

    /// 推进导演。档位或剧目变了返回 true。
    pub fn tick(&mut self, now_ms: u32) -> bool {
        let mut changed = false;
        let next_tier = self.tier_for(now_ms);
        if next_tier != self.tier {
            self.enter_tier(next_tier, now_ms);
            changed = true;
        }
        if self.tier == Tier::Alert {
            return changed;
        }
        let base = base_skit(self.tier);
        if self.skit != base && now_ms.wrapping_sub(self.skit_started_ms) as i32 >= self.skit.length_ms() as i32 {
            self.last_skit = self.skit;
            self.skit = base;
            self.skit_started_ms = now_ms;
            let gap = self.gap_for(self.tier);
            self.next_skit_at_ms = now_ms.wrapping_add(gap);
            changed = true;
        }
        if self.skit == base && now_ms.wrapping_sub(self.next_skit_at_ms) as i32 >= 0 {
            self.skit = self.pick_skit(self.tier);
            self.skit_started_ms = now_ms;
            changed = true;
        }
        changed
    }

    pub fn view(&self, now_ms: u32) -> View {
        View {
            tier: self.tier,
            skit: self.skit,
            skit_frame: now_ms.wrapping_sub(self.skit_started_ms) / FRAME_MS,
            dim: self.tier == Tier::Sleepy,
            lights_out: self.tier == Tier::Sleepy
                && self.is_night()
                && now_ms.wrapping_sub(self.idle_since_ms) >= LIGHTS_OUT_AFTER_MS,
        }
    }

    /// 测试与预览用：立刻开演某个剧目。
    pub fn start_skit(&mut self, skit: Skit, now_ms: u32) {
        let needed = if skit == Skit::Sleep { Tier::Sleepy } else { Tier::Bored };
        let back = if needed == Tier::Sleepy { SLEEPY_AFTER_MS } else { BORED_AFTER_MS };
        self.idle_since_ms = now_ms.wrapping_sub(back);
        self.enter_tier(needed, now_ms);
        self.skit = skit;
        self.skit_started_ms = now_ms;
        self.next_skit_at_ms = now_ms.wrapping_add(SLEEPY_GAP_MAX_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn minutes(n: u32) -> u32 {
        n * 60 * 1000
    }

    /// 把时间推到 now，每 20 ms tick 一次，像主循环那样。有符号比较，允许回绕。
    fn advance_to(leisure: &mut Leisure, clock: &mut u32, now: u32) {
        while now.wrapping_sub(*clock) as i32 > 0 {
            *clock = clock.wrapping_add(20);
            leisure.tick(*clock);
        }
    }

    fn is_sleep_family(skit: Skit) -> bool {
        matches!(skit, Skit::Sleep | Skit::Dream | Skit::Startle)
    }

    #[test]
    fn tiers_follow_idle_time() {
        let mut leisure = Leisure::new(7, 0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(4));
        assert_eq!(leisure.view(clock).tier, Tier::Alert);
        advance_to(&mut leisure, &mut clock, minutes(6));
        assert_eq!(leisure.view(clock).tier, Tier::Bored);
        assert!(!leisure.view(clock).dim);
        advance_to(&mut leisure, &mut clock, minutes(31));
        assert_eq!(leisure.view(clock).tier, Tier::Sleepy);
        assert!(leisure.view(clock).dim);
        assert!(is_sleep_family(leisure.view(clock).skit));
    }

    #[test]
    fn activity_resets_everything() {
        let mut leisure = Leisure::new(7, 0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(40));
        assert_eq!(leisure.view(clock).tier, Tier::Sleepy);
        leisure.note_activity(clock);
        leisure.tick(clock);
        let view = leisure.view(clock);
        assert_eq!(view.tier, Tier::Alert);
        assert_eq!(view.skit, Skit::None);
        assert!(!view.dim);
        assert!(!view.lights_out);
    }

    #[test]
    fn bored_plays_skits_and_never_repeats() {
        let mut leisure = Leisure::new(11, 0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(5) + 4000);
        // 进入无聊 3 秒后开第一场。
        assert_ne!(leisure.view(clock).skit, Skit::None);
        assert_ne!(leisure.view(clock).skit, Skit::Sleep);

        let mut previous = Skit::None;
        let mut in_gap = true;
        let mut played = 0;
        let mut repeats = 0;
        let mut seen = [false; SKIT_COUNT];
        let mut step = 0;
        while step < 4000 && clock < minutes(29) {
            step += 1;
            let target = clock + 500;
            advance_to(&mut leisure, &mut clock, target);
            let current = leisure.view(clock).skit;
            if current == Skit::None {
                in_gap = true;
                continue;
            }
            if !in_gap {
                continue; // 同一场还在演
            }
            in_gap = false;
            if current == previous {
                repeats += 1;
            }
            seen[current as usize] = true;
            previous = current;
            played += 1;
        }
        assert!(played >= 20);
        assert_eq!(repeats, 0);
        // 24 分钟里七出都该露过面。
        for skit in [Skit::Patrol, Skit::Ball, Skit::Read, Skit::Stars, Skit::Hide, Skit::Startle, Skit::Dream] {
            assert!(seen[skit as usize], "{skit:?} 没出现");
        }
    }

    #[test]
    fn skit_frames_advance_at_eight_fps() {
        let mut leisure = Leisure::new(3, 0);
        leisure.start_skit(Skit::Patrol, 1000);
        assert_eq!(leisure.view(1000).skit_frame, 0);
        assert_eq!(leisure.view(1000 + 125 * 8).skit_frame, 8);
        // 12 秒后巡逻结束，回到底色。
        let mut clock = 1000;
        advance_to(&mut leisure, &mut clock, 1000 + 12500);
        assert_eq!(leisure.view(clock).skit, Skit::None);
    }

    #[test]
    fn sleepy_only_dreams_or_startles() {
        let mut leisure = Leisure::new(5, 0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(31));
        for _ in 0..600 {
            let target = clock + 1000;
            advance_to(&mut leisure, &mut clock, target);
            assert!(is_sleep_family(leisure.view(clock).skit));
        }
    }

    #[test]
    fn lights_go_out_only_at_night() {
        let mut leisure = Leisure::new(9, 0);
        leisure.set_hour(14);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(100));
        assert_eq!(leisure.view(clock).tier, Tier::Sleepy);
        assert!(!leisure.view(clock).lights_out);

        // 22 点还不算夜里，23 点起算，到早上 7 点。
        leisure.set_hour(22);
        assert!(!leisure.view(clock).lights_out);
        leisure.set_hour(23);
        assert!(leisure.view(clock).lights_out);
        leisure.set_hour(3);
        assert!(leisure.view(clock).lights_out);
        leisure.set_hour(7);
        assert!(!leisure.view(clock).lights_out);
        leisure.set_hour(23);

        // 关着灯的时候有事，立刻亮。
        leisure.note_activity(clock);
        leisure.tick(clock);
        assert!(!leisure.view(clock).lights_out);

        // 夜里但没睡够 90 分钟，不关。
        let mut leisure = Leisure::new(9, 0);
        leisure.set_hour(1);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(60));
        assert_eq!(leisure.view(clock).tier, Tier::Sleepy);
        assert!(!leisure.view(clock).lights_out);
    }

    #[test]
    fn unknown_hour_is_daytime() {
        let mut leisure = Leisure::new(9, 0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(200));
        assert!(!leisure.view(clock).lights_out);
    }

    #[test]
    fn force_bored_starts_playing_now() {
        let mut leisure = Leisure::new(13, 0);
        let mut clock = 1000;
        leisure.force_bored(clock);
        leisure.tick(clock);
        assert_eq!(leisure.view(clock).tier, Tier::Bored);
        let target = clock + 4000;
            advance_to(&mut leisure, &mut clock, target);
        assert_ne!(leisure.view(clock).skit, Skit::None);
    }

    #[test]
    fn nothing_done_means_kicking_the_ball() {
        let mut leisure = Leisure::new(17, 0);
        leisure.set_done_count(0);
        let mut clock = 0;
        advance_to(&mut leisure, &mut clock, minutes(5) + 4000);
        let mut ball = 0;
        let mut played = 0;
        let mut previous = Skit::None;
        while clock < minutes(29) {
            let target = clock + 500;
            advance_to(&mut leisure, &mut clock, target);
            let current = leisure.view(clock).skit;
            if current == Skit::None || current == previous {
                continue;
            }
            previous = current;
            played += 1;
            if current == Skit::Ball {
                ball += 1;
            }
        }
        // 权重 7/22，即便不连演，也该占三成上下。
        assert!(played >= 20);
        assert!(ball * 10 >= played * 2);
    }

    #[test]
    fn millisecond_counter_may_wrap() {
        let start = u32::MAX - minutes(2);
        let mut leisure = Leisure::new(19, start);
        let mut clock = start;
        advance_to(&mut leisure, &mut clock, start.wrapping_add(minutes(4)));
        assert_eq!(leisure.view(clock).tier, Tier::Alert);
        advance_to(&mut leisure, &mut clock, start.wrapping_add(minutes(6)));
        assert_eq!(leisure.view(clock).tier, Tier::Bored);
    }
}
