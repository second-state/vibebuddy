//! Special occasions: the rarer readings of a done or needs-input that make the buddy pick its line
//! from another pool (docs/characters.md). The daemon decides them because it owns today's stats,
//! the clock and the link; the box only draws a line from the pool it is told.

use chrono::{Days, NaiveDate, NaiveDateTime, TimeDelta, Timelike};

/// The completion counts of a day that are worth a milestone line.
const MILESTONES: [u32; 3] = [5, 10, 20];
/// The busy hours of a day that are worth a long-session line; past the last, no more.
const LONG_SESSION_HOURS: u64 = 3;
/// How long the Agents must have been quiet for the next activity to be a welcome back.
pub const WELCOME_BACK_QUIET: TimeDelta = TimeDelta::hours(3);

/// How many whole busy hours of the day deserve a long-session line: 0 to 3.
pub fn long_session_hours(busy_seconds: u64) -> u64 {
    (busy_seconds / 3600).min(LONG_SESSION_HOURS)
}

/// Late night runs from 23:00 to 05:00 the next morning, and belongs to the evening it started on.
pub fn night_of(now: NaiveDateTime) -> Option<NaiveDate> {
    match now.hour() {
        23 => Some(now.date()),
        0..5 => now.date().checked_sub_days(Days::new(1)),
        _ => None,
    }
}

/// The special occasion of a done, if any: the rarest that fits wins. `done_today` already counts
/// this one; `late_night_used` is the night whose late-night line has been spent; `long_session_due`
/// says a new busy hour has passed since the last long-session line.
pub fn done_occasion(now: NaiveDateTime, done_today: u32, late_night_used: Option<NaiveDate>, long_session_due: bool) -> Option<&'static str> {
    if night_of(now).is_some_and(|night| Some(night) != late_night_used) {
        Some("late_night_done")
    } else if done_today == 1 {
        Some("first_done")
    } else if long_session_due {
        Some("long_session")
    } else if MILESTONES.contains(&done_today) {
        Some("milestone")
    } else {
        None
    }
}

/// The special occasion of a needs input: only late night has one.
pub fn input_occasion(now: NaiveDateTime, late_night_used: Option<NaiveDate>) -> Option<&'static str> {
    night_of(now).is_some_and(|night| Some(night) != late_night_used).then_some("late_night_input")
}

/// Which greeting the first link of the day gets, by the time of day.
pub fn greeting(now: NaiveDateTime) -> &'static str {
    match now.hour() {
        5..12 => "greeting_morning",
        12..18 => "greeting_afternoon",
        _ => "greeting_evening",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(hour, 30, 0).unwrap()
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn a_night_spans_midnight() {
        assert_eq!(night_of(at(6, 22)), None);
        assert_eq!(night_of(at(6, 23)), Some(date(6)));
        assert_eq!(night_of(at(7, 0)), Some(date(6)));
        assert_eq!(night_of(at(7, 4)), Some(date(6)));
        assert_eq!(night_of(at(7, 5)), None);
    }

    #[test]
    fn the_rarest_occasion_wins() {
        assert_eq!(done_occasion(at(6, 10), 1, None, false), Some("first_done"));
        assert_eq!(done_occasion(at(6, 10), 2, None, false), None);
        assert_eq!(done_occasion(at(6, 10), 5, None, false), Some("milestone"));
        assert_eq!(done_occasion(at(6, 10), 10, None, false), Some("milestone"));
        assert_eq!(done_occasion(at(6, 10), 11, None, false), None);
        // Late night beats first done, once a night.
        assert_eq!(done_occasion(at(6, 23), 1, None, false), Some("late_night_done"));
        assert_eq!(done_occasion(at(7, 1), 1, Some(date(6)), false), Some("first_done"));
        assert_eq!(done_occasion(at(7, 23), 3, Some(date(6)), false), Some("late_night_done"), "a new night");
    }

    #[test]
    fn a_long_session_beats_a_milestone_but_not_the_first_done() {
        assert_eq!(done_occasion(at(6, 15), 5, None, true), Some("long_session"));
        assert_eq!(done_occasion(at(6, 15), 1, None, true), Some("first_done"));
        assert_eq!(done_occasion(at(6, 23), 7, None, true), Some("late_night_done"));
        assert_eq!(long_session_hours(3599), 0);
        assert_eq!(long_session_hours(2 * 3600 + 5), 2);
        assert_eq!(long_session_hours(9 * 3600), 3, "no more after three hours");
    }

    #[test]
    fn needs_input_only_has_late_night() {
        assert_eq!(input_occasion(at(6, 15), None), None);
        assert_eq!(input_occasion(at(7, 2), None), Some("late_night_input"));
        assert_eq!(input_occasion(at(7, 2), Some(date(6))), None);
    }

    #[test]
    fn greetings_follow_the_time_of_day() {
        assert_eq!(greeting(at(6, 4)), "greeting_evening");
        assert_eq!(greeting(at(6, 5)), "greeting_morning");
        assert_eq!(greeting(at(6, 12)), "greeting_afternoon");
        assert_eq!(greeting(at(6, 18)), "greeting_evening");
    }
}
