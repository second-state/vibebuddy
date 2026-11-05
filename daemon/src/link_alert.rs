//! Outside macOS there is no app to say the box is gone, so the daemon says it: with `notify-send` on Linux, a toast
//! on Windows.
//! Same rule as the app: once, after 30 seconds without the box, and again only after it has come back.

use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::warn;

const GRACE: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
const TITLE: &str = "Box disconnected";
const BODY: &str = "VibeBuddy hasn't seen the box for 30 seconds. Check the USB cable.";

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
        if alert.check(connected, enabled, Instant::now())
            && let Err(error) = notify().status().await
        {
            warn!(%error, "cannot show the notification about the lost link");
        }
    }
}

#[cfg(not(windows))]
fn notify() -> Command {
    let mut command = Command::new("notify-send");
    command.args(["--app-name=VibeBuddy", TITLE, BODY]);
    command
}

/// Windows PowerShell shows the toast: under VibeBuddy's app id once the installer's Start menu shortcut carries it,
/// else under PowerShell's own, since an app needs an installed shortcut for an id of its own.
/// The text goes in through the environment, so nothing in it is read as script.
#[cfg(windows)]
fn notify() -> Command {
    const SCRIPT: &str = "$m = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]; \
        $x = $m::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
        $t = $x.GetElementsByTagName('text'); \
        [void]$t.Item(0).AppendChild($x.CreateTextNode($env:VIBEBUDDY_TITLE)); \
        [void]$t.Item(1).AppendChild($x.CreateTextNode($env:VIBEBUDDY_BODY)); \
        $id = if (Test-Path \"$env:APPDATA\\Microsoft\\Windows\\Start Menu\\Programs\\VibeBuddy.lnk\") { 'SecondState.VibeBuddy' } \
            else { '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe' }; \
        $m::CreateToastNotifier($id).Show([Windows.UI.Notifications.ToastNotification]::new($x))";
    // No console window flashes up for it.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("VIBEBUDDY_TITLE", TITLE)
        .env("VIBEBUDDY_BODY", BODY)
        .creation_flags(CREATE_NO_WINDOW);
    command
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
