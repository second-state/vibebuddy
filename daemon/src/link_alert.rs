//! Outside macOS there is no app to say the box is gone, so the daemon says it with `notify-send`.
//! Same rule as the app: once, after 30 seconds without the box, and again only after it has come back.

use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::warn;

const GRACE: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TITLE: &str = "Box disconnected";
const BODY: &str = "Vibe Buddy hasn't seen the box for 30 seconds. Check the USB cable.";

#[derive(Default)]
struct LinkAlert {
    lost_since: Option<Instant>,
    notified: bool,
}

impl LinkAlert {
    /// Returns true exactly when the notification should go out.
    fn check(&mut self, connected: bool, enabled: bool, now: Instant) -> bool {
        if connected {
            self.lost_since = None;
            self.notified = false;
            return false;
        }
        if !enabled {
            self.lost_since = None;
            return false;
        }
        let since = *self.lost_since.get_or_insert(now);
        if self.notified || now.duration_since(since) < GRACE {
            return false;
        }
        self.notified = true;
        true
    }
}

pub(crate) async fn watch(state: crate::AppState) {
    let mut alert = LinkAlert::default();
    let mut ticker = tokio::time::interval(CHECK_INTERVAL);
    loop {
        ticker.tick().await;
        let connected = state.device.lock().await.connected;
        let enabled = state.config.lock().await.notify_link;
        if alert.check(connected, enabled, Instant::now()) {
            let result = Command::new("notify-send")
                .args(["--app-name=Vibe Buddy", TITLE, BODY])
                .status()
                .await;
            if let Err(error) = result {
                warn!(%error, "cannot run notify-send to report the lost link");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifies_once_after_the_grace_period_and_rearms_on_reconnect() {
        let start = Instant::now();
        let mut alert = LinkAlert::default();
        assert!(!alert.check(false, true, start));
        assert!(!alert.check(false, true, start + Duration::from_secs(29)));
        assert!(alert.check(false, true, start + GRACE));
        assert!(!alert.check(false, true, start + Duration::from_secs(120)));
        assert!(!alert.check(true, true, start + Duration::from_secs(121)));
        let lost_again = start + Duration::from_secs(200);
        assert!(!alert.check(false, true, lost_again));
        assert!(alert.check(false, true, lost_again + GRACE));
    }

    #[test]
    fn a_short_blip_does_not_notify() {
        let start = Instant::now();
        let mut alert = LinkAlert::default();
        assert!(!alert.check(false, true, start));
        assert!(!alert.check(true, true, start + Duration::from_secs(10)));
        assert!(!alert.check(false, true, start + Duration::from_secs(35)));
    }

    #[test]
    fn turning_the_setting_off_silences_it() {
        let start = Instant::now();
        let mut alert = LinkAlert::default();
        assert!(!alert.check(false, false, start));
        assert!(!alert.check(false, false, start + Duration::from_secs(60)));
        // Turned back on: the 30 seconds count from then, not from when the box left.
        assert!(!alert.check(false, true, start + Duration::from_secs(61)));
        assert!(alert.check(false, true, start + Duration::from_secs(91)));
    }
}
