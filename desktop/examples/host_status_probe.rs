//! Headless test consumer of the Linux app's real status and tray model. One JSON status per stdin line;
//! one derived menu per stdout line. This doesn't start iced, a tray host, or a desktop service.

#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../src/status.rs"]
mod status;

use std::io::{BufRead, Write};

// The probe always uses English and never reads/writes the user's saved language.
fn config_dir() -> Option<std::path::PathBuf> {
    None
}

fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("read status line");
        let snapshot: Option<status::Status> =
            serde_json::from_str(&line).expect("decode app status");
        let menu = status::MenuState::derive(snapshot.as_ref());
        let icon = match menu.icon {
            status::Icon::Online => "online",
            status::Icon::Offline => "offline",
            status::Icon::DaemonDown => "daemon_down",
        };
        println!(
            "{}",
            serde_json::json!({
                "icon": icon,
                "device_line": menu.device_line,
                "mode_line": menu.mode_line,
                "today_line": menu.today_line,
                "volume": snapshot.as_ref().and_then(|value| value.device.volume),
            })
        );
        std::io::stdout().flush().expect("flush derived menu");
    }
}
