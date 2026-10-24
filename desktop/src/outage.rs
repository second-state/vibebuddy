//! Tells the user when the daemon fails and doesn't come back, as the Mac's app does when its supervisor gives up
//! (`AppModel.swift`, `onGiveUp`). The decision is a small state machine that takes explicit instants and events, so
//! `main.rs` and the headless `daemon_outage_probe` drive the same code. The adapters at the bottom, `service` and
//! `notify`, are the ones both use.

use std::time::{Duration, Instant};

use crate::i18n::tr;
use crate::status::Supervisor;

/// How long the daemon must stay unreachable before anyone is told: long enough to skip a restart after a crash.
pub const GRACE: Duration = Duration::from_secs(30);

/// How long `systemctl` may take before the unit's state counts as unknown.
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Active,
    Activating,
    Failed,
    /// Stopped on purpose, or not installed: `is-active` says `inactive` for both, so the user's stop wins.
    Inactive,
    /// `systemctl` gave no state: no user bus, no binary, or it timed out. The user is told, since nothing else is known.
    Unknown,
}

impl Service {
    /// `systemctl --user is-active` prints one state word; any other output is unknown.
    pub fn parse(output: &str) -> Self {
        match output.trim() {
            "active" | "reloading" => Self::Active,
            "activating" => Self::Activating,
            "failed" => Self::Failed,
            "inactive" | "deactivating" => Self::Inactive,
            _ => Self::Unknown,
        }
    }

    fn suppresses(self) -> bool {
        self == Self::Inactive
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A status arrived: the daemon answers, with its notify toggle and who supervises it.
    Status { notify_link: bool, supervisor: Option<Supervisor> },
    /// The status stream couldn't reach the daemon. It reports this again on every retry.
    Down,
    /// The grace period has passed since the outage began.
    GraceElapsed,
    /// The unit's state, read after the grace period.
    Service(Service),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Send `Event::GraceElapsed` after this long.
    Wait(Duration),
    /// Read the unit's state, then send `Event::Service`.
    CheckService,
    /// Tell the user, saying who can bring the daemon back.
    Notify(Restart),
}

/// Who can start the daemon again, as the notification says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// The app, from the tray or Advanced, through the systemd service.
    Here,
    /// The user: the daemon didn't run as the service, so its unit's state says nothing and no button restarts it.
    Yourself,
}

pub struct Outage {
    grace: Duration,
    /// When the current outage began: the first `Down` since the daemon last answered.
    down_since: Option<Instant>,
    notified: bool,
    /// The toggle from the last status the daemon sent. It survives the daemon going away; on until one was read.
    notify_link: bool,
    /// Who supervised the daemon, from the last status that said; unknown until then, as from an older daemon.
    supervisor: Option<Supervisor>,
}

impl Outage {
    pub fn new(grace: Duration) -> Self {
        Self {
            grace,
            down_since: None,
            notified: false,
            notify_link: true,
            supervisor: None,
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Option<Effect> {
        match event {
            Event::Status { notify_link, supervisor } => {
                self.down_since = None;
                self.notified = false;
                self.notify_link = notify_link;
                self.supervisor = supervisor.or(self.supervisor);
                None
            }
            Event::Down => {
                if self.down_since.is_some() {
                    return None;
                }
                self.down_since = Some(now);
                Some(Effect::Wait(self.grace))
            }
            Event::GraceElapsed if !self.due(now) => None,
            // The service's state is about another daemon, or none: tell the user without asking systemd.
            Event::GraceElapsed if self.supervisor == Some(Supervisor::Other) => {
                self.notified = true;
                Some(Effect::Notify(Restart::Yourself))
            }
            Event::GraceElapsed => Some(Effect::CheckService),
            Event::Service(service) => {
                if !self.due(now) {
                    return None;
                }
                // A stopped unit can later fail to start without ever serving a status snapshot. Keep observing it
                // so intentional-stop suppression does not hide that later failure forever.
                if service.suppresses() {
                    return Some(Effect::Wait(self.grace));
                }
                self.notified = true;
                Some(Effect::Notify(Restart::Here))
            }
        }
    }

    /// The outage has lasted the grace period, nobody has been told, and the user wants to be told.
    fn due(&self, now: Instant) -> bool {
        self.down_since.is_some_and(|since| now.duration_since(since) >= self.grace) && !self.notified && self.notify_link
    }
}

/// The unit's state from `systemctl --user is-active`. It exits non-zero for every state but `active`, so only the
/// printed word counts.
pub async fn service() -> Service {
    let command = tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "vibebuddyd"])
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(SYSTEMCTL_TIMEOUT, command).await {
        Ok(Ok(output)) => Service::parse(&String::from_utf8_lossy(&output.stdout)),
        _ => Service::Unknown,
    }
}

/// The notification, sent with `notify-send` as the firmware one is. It is spawned and not awaited, so a slow
/// notification server never holds up the app.
pub fn notify(restart: Restart) -> Result<(), String> {
    let body = match restart {
        Restart::Here => tr("The daemon stopped and didn't come back. Restart it from the tray icon or on the Advanced tab.", &[]),
        Restart::Yourself => tr(
            "The daemon stopped and didn't come back. It didn't run as the vibebuddyd systemd service, so start it again the way you started it.",
            &[],
        ),
    };
    std::process::Command::new("notify-send")
        .args(["--app-name=VibeBuddy", "VibeBuddy", body.as_str()])
        .spawn()
        .map(drop)
        .map_err(|error| format!("cannot run notify-send: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, seconds: f64) -> Instant {
        start + Duration::from_secs_f64(seconds)
    }

    fn outage() -> Outage {
        Outage::new(GRACE)
    }

    /// A status from a daemon the systemd service runs.
    fn status(notify_link: bool) -> Event {
        Event::Status { notify_link, supervisor: Some(Supervisor::Systemd) }
    }

    #[test]
    fn a_daemon_outside_the_service_notifies_without_asking_systemd() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Status { notify_link: true, supervisor: Some(Supervisor::Other) }, start);
        assert_eq!(policy.handle(Event::Down, at(start, 1.0)), Some(Effect::Wait(GRACE)));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 31.0)), Some(Effect::Notify(Restart::Yourself)));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 62.0)), None);
    }

    #[test]
    fn the_supervisor_survives_a_status_from_an_older_daemon() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Status { notify_link: true, supervisor: Some(Supervisor::Other) }, start);
        policy.handle(Event::Status { notify_link: true, supervisor: None }, at(start, 1.0));
        policy.handle(Event::Down, at(start, 2.0));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 32.0)), Some(Effect::Notify(Restart::Yourself)));
    }

    #[test]
    fn with_no_supervisor_known_the_service_is_asked() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), Some(Effect::CheckService));
    }

    #[test]
    fn the_grace_boundary_is_exact() {
        let start = Instant::now();
        let mut policy = outage();
        assert_eq!(policy.handle(Event::Down, start), Some(Effect::Wait(GRACE)));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 29.999)), None);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), Some(Effect::CheckService));
    }

    #[test]
    fn a_short_outage_never_notifies() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        assert_eq!(policy.handle(status(true), at(start, 5.0)), None);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), None);
    }

    #[test]
    fn repeated_failures_keep_the_first_instant_and_one_timer() {
        let start = Instant::now();
        let mut policy = outage();
        assert_eq!(policy.handle(Event::Down, start), Some(Effect::Wait(GRACE)));
        assert_eq!(policy.handle(Event::Down, at(start, 2.0)), None);
        assert_eq!(policy.handle(Event::Down, at(start, 4.0)), None);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), Some(Effect::CheckService));
    }

    #[test]
    fn a_sustained_outage_notifies_once() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        assert_eq!(
            policy.handle(Event::Service(Service::Failed), at(start, 30.0)),
            Some(Effect::Notify(Restart::Here))
        );
        assert_eq!(policy.handle(Event::Down, at(start, 32.0)), None);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 60.0)), None);
    }

    #[test]
    fn recovery_rearms_the_notification_for_the_next_outage() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        policy.handle(Event::Service(Service::Failed), at(start, 30.0));
        policy.handle(status(true), at(start, 40.0));
        assert_eq!(policy.handle(Event::Down, at(start, 50.0)), Some(Effect::Wait(GRACE)));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 80.0)), Some(Effect::CheckService));
        assert_eq!(
            policy.handle(Event::Service(Service::Failed), at(start, 80.0)),
            Some(Effect::Notify(Restart::Here))
        );
    }

    #[test]
    fn a_timer_from_an_earlier_outage_does_not_decide_a_later_one() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(status(true), at(start, 5.0));
        policy.handle(Event::Down, at(start, 10.0));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), None);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 40.0)), Some(Effect::CheckService));
    }

    #[test]
    fn a_state_read_after_the_daemon_returned_decides_nothing() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        policy.handle(status(true), at(start, 31.0));
        assert_eq!(policy.handle(Event::Service(Service::Failed), at(start, 31.0)), None);
    }

    #[test]
    fn turning_the_notification_off_silences_it() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(status(false), start);
        policy.handle(Event::Down, start);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), None);
    }

    #[test]
    fn the_last_known_toggle_outlives_the_daemon() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(status(false), start);
        policy.handle(Event::Down, at(start, 1.0));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 31.0)), None);
    }

    #[test]
    fn without_any_status_the_notification_is_on() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 30.0)), Some(Effect::CheckService));
    }

    #[test]
    fn an_inactive_unit_is_a_stop_the_user_asked_for() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        assert_eq!(
            policy.handle(Event::Service(Service::Inactive), at(start, 30.0)),
            Some(Effect::Wait(GRACE))
        );
    }

    #[test]
    fn a_failed_restart_after_an_intentional_stop_notifies_without_a_status() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        policy.handle(Event::Service(Service::Inactive), at(start, 30.0));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 60.0)), Some(Effect::CheckService));
        assert_eq!(
            policy.handle(Event::Service(Service::Failed), at(start, 60.0)),
            Some(Effect::Notify(Restart::Here))
        );
        assert_eq!(policy.handle(Event::Down, at(start, 62.0)), None);
    }

    #[test]
    fn recovery_cancels_checks_scheduled_while_a_unit_was_stopped() {
        let start = Instant::now();
        let mut policy = outage();
        policy.handle(Event::Down, start);
        policy.handle(Event::GraceElapsed, at(start, 30.0));
        policy.handle(Event::Service(Service::Inactive), at(start, 30.0));
        policy.handle(status(true), at(start, 40.0));
        assert_eq!(policy.handle(Event::GraceElapsed, at(start, 60.0)), None);
    }

    #[test]
    fn every_other_unit_state_notifies() {
        for service in [Service::Active, Service::Activating, Service::Failed, Service::Unknown] {
            let start = Instant::now();
            let mut policy = outage();
            policy.handle(Event::Down, start);
            policy.handle(Event::GraceElapsed, at(start, 30.0));
            assert_eq!(
                policy.handle(Event::Service(service), at(start, 30.0)),
                Some(Effect::Notify(Restart::Here)),
                "{service:?}"
            );
        }
    }

    #[test]
    fn systemctl_output_maps_to_a_unit_state() {
        assert_eq!(Service::parse("active\n"), Service::Active);
        assert_eq!(Service::parse("activating\n"), Service::Activating);
        assert_eq!(Service::parse("failed\n"), Service::Failed);
        assert_eq!(Service::parse("inactive\n"), Service::Inactive);
        assert_eq!(Service::parse("deactivating\n"), Service::Inactive);
        assert_eq!(Service::parse(""), Service::Unknown);
        assert_eq!(Service::parse("Failed to connect to bus\n"), Service::Unknown);
    }
}
