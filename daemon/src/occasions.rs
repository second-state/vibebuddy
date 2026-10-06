//! Special occasions: the rarer readings of a done or needs-input that make the buddy pick its line
//! from another pool (docs/characters.md). The daemon decides them because it owns today's stats,
//! the clock and the link; the box only draws a line from the pool it is told.

use chrono::{Days, NaiveDate, NaiveDateTime, Timelike};

/// The completion counts of a day that are worth a milestone line.
const MILESTONES: [u32; 3] = [5, 10, 20];

/// Late night runs from 23:00 to 05:00 the next morning, and belongs to the evening it started on.
pub fn night_of(now: NaiveDateTime) -> Option<NaiveDate> {
    match now.hour() {
        23 => Some(now.date()),
        0..5 => now.date().checked_sub_days(Days::new(1)),
        _ => None,
    }
}

/// The special occasion of a done, if any: the rarest that fits wins. `done_today` already counts
/// this one; `late_night_used` is the night whose late-night line has been spent.
pub fn done_occasion(now: NaiveDateTime, done_today: u32, late_night_used: Option<NaiveDate>) -> Option<&'static str> {
    if night_of(now).is_some_and(|night| Some(night) != late_night_used) {
        Some("late_night_done")
    } else if done_today == 1 {
        Some("first_done")
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
        assert_eq!(done_occasion(at(6, 10), 1, None), Some("first_done"));
        assert_eq!(done_occasion(at(6, 10), 2, None), None);
        assert_eq!(done_occasion(at(6, 10), 5, None), Some("milestone"));
        assert_eq!(done_occasion(at(6, 10), 10, None), Some("milestone"));
        assert_eq!(done_occasion(at(6, 10), 11, None), None);
        // Late night beats first done, once a night.
        assert_eq!(done_occasion(at(6, 23), 1, None), Some("late_night_done"));
        assert_eq!(done_occasion(at(7, 1), 1, Some(date(6))), Some("first_done"));
        assert_eq!(done_occasion(at(7, 23), 3, Some(date(6))), Some("late_night_done"), "a new night");
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
