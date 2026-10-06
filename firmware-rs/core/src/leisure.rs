//! Director for leisure mode: tracks boredom, draws skits, and decides when to dim and
//! turn off the backlight. It reads no clock and touches no hardware; every entry point
//! takes a millisecond count from the caller, and wraparound is allowed.

use crate::TIME_SCALE;

/// How long idle counts as bored, as sleepy, and how long sleepy at night before the backlight goes off.
pub const BORED_AFTER_MS: u32 = 5 * 60 * 1000 / TIME_SCALE;
pub const SLEEPY_AFTER_MS: u32 = 30 * 60 * 1000 / TIME_SCALE;
pub const LIGHTS_OUT_AFTER_MS: u32 = 90 * 60 * 1000 / TIME_SCALE;
/// Frame length of skit animations: 8 fps.
pub const FRAME_MS: u32 = 125;

/// How long after entering a level the first skit starts; later gaps are random per level.
const FIRST_SKIT_DELAY_MS: u32 = 3000;
const BORED_GAP_MIN_MS: u32 = 20000 / TIME_SCALE;
const BORED_GAP_MAX_MS: u32 = 40000 / TIME_SCALE;
const SLEEPY_GAP_MIN_MS: u32 = 120_000 / TIME_SCALE;
const SLEEPY_GAP_MAX_MS: u32 = 300_000 / TIME_SCALE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Standby: the usual breathing, blinking and rotating stats.
    Alert,
    /// Bored: play a short skit every so often.
    Bored,
    /// Sleepy: mostly sleeping, with the screen dimmed.
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
    /// Plain idle between skits.
    None,
    Patrol,
    Ball,
    Read,
    Stars,
    Hide,
    Startle,
    Dream,
    /// Base look while sleepy: sleeping.
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

    /// Length of each skit; 0 means it plays until the level changes.
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
    /// How many frames of the current skit have played.
    pub skit_frame: u32,
    /// Sleepy: dim the screen.
    pub dim: bool,
    /// Asleep long enough at night: backlight off.
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
        // xorshift32: random enough, and small enough.
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

    /// Night: 23:00 to 07:00. An unknown hour counts as daytime: better to stay lit than go dark in the afternoon.
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

    /// Skit weights. At night, more sleep and less play; nothing done today means bored ball-kicking, lots done means tired and more dreams.
    fn skit_weights(&self, tier: Tier) -> [u32; SKIT_COUNT] {
        let mut weights = [0u32; SKIT_COUNT];
        let mut set = |skit: Skit, weight: u32| weights[skit as usize] = weight;
        if tier == Tier::Sleepy {
            set(Skit::Dream, if self.done_count >= 5 { 4 } else { 3 });
            set(Skit::Startle, if self.done_count >= 5 { 0 } else { 2 });
            return weights;
        }
        let table: [(Skit, u32, u32); 7] = [
            // skit, night weight, day weight
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
        // Don't play the same skit twice in a row; repeat only when it is the only choice left.
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

    /// Any activity resets boredom: agent activity, a key press, a running pomodoro.
    pub fn note_activity(&mut self, now_ms: u32) {
        self.idle_since_ms = now_ms;
    }

    /// Local hour from the Mac's heartbeat; -1 means unknown.
    pub fn set_hour(&mut self, hour: i32) {
        self.hour = hour;
    }

    /// Focus sessions or tasks completed today, which decides whether it is tired or bored.
    pub fn set_done_count(&mut self, done: u32) {
        self.done_count = done;
    }

    /// Advances the director. Returns true when the level or skit changed.
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

    /// For tests and previews: start a given skit immediately.
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

    /// Advances time to now, ticking every 20 ms like the main loop. Signed comparison, so wraparound is allowed.
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
        // The first skit starts 3 seconds after becoming bored.
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
                continue; // the same skit is still playing
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
        // All seven skits should show up within 24 minutes.
        for skit in [Skit::Patrol, Skit::Ball, Skit::Read, Skit::Stars, Skit::Hide, Skit::Startle, Skit::Dream] {
            assert!(seen[skit as usize], "{skit:?} never appeared");
        }
    }

    #[test]
    fn skit_frames_advance_at_eight_fps() {
        let mut leisure = Leisure::new(3, 0);
        leisure.start_skit(Skit::Patrol, 1000);
        assert_eq!(leisure.view(1000).skit_frame, 0);
        assert_eq!(leisure.view(1000 + 125 * 8).skit_frame, 8);
        // Patrol ends after 12 seconds and returns to the base look.
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

        // 22:00 is not night yet; night runs from 23:00 to 07:00.
        leisure.set_hour(22);
        assert!(!leisure.view(clock).lights_out);
        leisure.set_hour(23);
        assert!(leisure.view(clock).lights_out);
        leisure.set_hour(3);
        assert!(leisure.view(clock).lights_out);
        leisure.set_hour(7);
        assert!(!leisure.view(clock).lights_out);
        leisure.set_hour(23);

        // Activity while the lights are out turns them on immediately.
        leisure.note_activity(clock);
        leisure.tick(clock);
        assert!(!leisure.view(clock).lights_out);

        // Night, but not asleep for 90 minutes yet: stay on.
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
        // Weight 7/22: even without back-to-back repeats it should be around 30%.
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
