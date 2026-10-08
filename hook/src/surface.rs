//! Decides where the agent process runs: in its own desktop app, inside another app (terminal,
//! editor), or with no host at all.
//!
//! The only evidence is `__CFBundleIdentifier`, injected by LaunchServices when it launches an app and inherited down the
//! process chain to the hook. `TERM_PROGRAM` isn't used: it would need a table mapping terminal names to bundle ids,
//! which breaks as soon as the user switches to an unfamiliar terminal. Nor the tty: agents run tool commands in
//! non-interactive child processes that report not a tty even when the host is a terminal (verified by running
//! codex in Ghostty on 2026-09-21).
//!
//! Some apps scrub that variable before starting the agent: since the Codex desktop app became
//! ChatGPT.app (26.924, 2026-09), its `codex app-server` runs without it, so every Codex session
//! looked headless and K2 had nowhere to go. When the variable is missing, the hook walks up its
//! own process chain to the ancestor launchd started — the process LaunchServices would have
//! named — and reads the bundle id of the outermost `.app` it lives in. Outermost, because
//! ChatGPT.app nests `CodexCLI.app` (`com.openai.codex.cli`) inside itself, and that inner bundle
//! has no window to bring forward.
//!
//! A terminal host also gets where in it the session sits, so the daemon can reach the session's own tab.
//! Normally that is the agent's tty (`ttys016`), the one thing that tells two tabs in the same directory
//! apart, read from the process table, walking up from the hook in case the agent started it in a session
//! of its own. Inside tmux that tty belongs to a pane, so the tmux socket and pane are reported instead.
//!
//! Outside macOS there is no such variable, so the hook reports its ancestor pids instead and leaves
//! finding their window to the daemon, which asks the compositor when K2 is pressed.

use std::collections::HashMap;
use std::process::Command;

use serde_json::{Map, Value};

const BUNDLE_ID: &str = "__CFBundleIdentifier";

#[derive(Debug, PartialEq, Eq)]
pub enum Surface {
    /// Runs in the agent's own desktop app; K2 uses that agent's deeplink.
    App,
    /// Runs inside another app, and this bundle id is where K2 goes. Unknown apps are all treated
    /// as hosts, so even unfamiliar terminals are reached correctly. The tab, when it can be told,
    /// narrows it down.
    Host(String, Option<Tab>),
    /// No host app: sessions started over SSH, by a daemon, or by launchd. K2 has nowhere to go.
    Headless,
    /// Outside macOS there are no bundle ids: the hook's ancestor process ids, nearest first. The daemon
    /// looks for a window owned by one of them when K2 is pressed; finding none means headless.
    Window(Vec<u32>),
}

/// Where in a terminal host the session sits.
#[derive(Debug, PartialEq, Eq)]
pub enum Tab {
    /// Nearest first.
    Ttys(Vec<String>),
    Tmux { socket: String, pane: String },
}

pub fn detect(own_bundle_id: &str) -> Surface {
    if !cfg!(target_os = "macos") {
        return Surface::Window(ancestor_pids());
    }
    let found = std::env::var(BUNDLE_ID)
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(launched_app_bundle_id);
    match from_bundle_id(found.as_deref(), own_bundle_id) {
        Surface::Host(bundle_id, _) => Surface::Host(bundle_id, current_tab()),
        surface => surface,
    }
}

fn current_tab() -> Option<Tab> {
    let tmux = std::env::var("TMUX").ok();
    let pane = std::env::var("TMUX_PANE").ok();
    match (tmux, pane) {
        (Some(tmux), Some(pane)) => tmux_tab(&tmux, &pane),
        _ => Some(controlling_ttys()).filter(|ttys| !ttys.is_empty()).map(Tab::Ttys),
    }
}

/// `$TMUX` is `<socket>,<server pid>,<session index>`; the socket path itself may hold commas.
fn tmux_tab(tmux: &str, pane: &str) -> Option<Tab> {
    let socket = tmux.rsplitn(3, ',').nth(2)?;
    let valid_pane = pane.strip_prefix('%').is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
    (socket.starts_with('/') && valid_pane).then(|| Tab::Tmux { socket: socket.to_owned(), pane: pane.to_owned() })
}

/// The controlling terminals up this process chain, nearest first, as `ttys016`. There can be more than one:
/// a terminal wrapper such as `kiro-cli-term` or `q term` runs the shell on a pseudo-terminal of its own, so
/// the agent's tty is not the one the terminal app names for its tab, which is further up.
#[cfg(target_os = "macos")]
fn controlling_ttys() -> Vec<String> {
    const NO_DEVICE: u32 = u32::MAX;
    let mut ttys: Vec<String> = Vec::new();
    let mut pid = std::process::id() as libc::c_int;
    for _ in 0..32 {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let read = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
        if read != size {
            break;
        }
        if info.e_tdev != NO_DEVICE {
            let name = unsafe { libc::devname(info.e_tdev as libc::dev_t, libc::S_IFCHR) };
            if !name.is_null()
                && let Ok(name) = unsafe { std::ffi::CStr::from_ptr(name) }.to_str()
                && name.starts_with("tty")
                && !ttys.iter().any(|tty| tty == name)
            {
                ttys.push(name.to_owned());
            }
        }
        pid = info.pbi_ppid as libc::c_int;
        if pid <= 1 {
            break;
        }
    }
    ttys
}

#[cfg(not(target_os = "macos"))]
fn controlling_ttys() -> Vec<String> {
    Vec::new()
}

/// The bundle id of the app launchd started at the top of this process chain, if it is an app.
/// Only runs when `__CFBundleIdentifier` is missing, so the common path spawns nothing.
fn launched_app_bundle_id() -> Option<String> {
    let table = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid=,comm="]).output().ok()?;
    let table = String::from_utf8(table.stdout).ok()?;
    let app = launched_app(&table, std::process::id())?;
    let plist = format!("{app}/Contents/Info.plist");
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-", &plist])
        .output()
        .ok()?;
    let id = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !id.is_empty()).then_some(id)
}

/// Walks `ps -A -o pid=,ppid=,comm=` output from `pid` up to the ancestor whose parent is launchd,
/// and returns the outermost `.app` bundle its executable lives in. SSH sessions, tmux servers and
/// launchd jobs top out at a plain binary and give `None`.
fn launched_app(table: &str, pid: u32) -> Option<String> {
    let processes: HashMap<u32, (u32, &str)> = table
        .lines()
        .filter_map(|line| {
            // Columns are padded with runs of spaces, and the path may contain spaces itself.
            let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let (ppid, path) = rest.trim_start().split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, (ppid.parse().ok()?, path.trim())))
        })
        .collect();
    let mut current = pid;
    // A cycle cannot happen in a real process table; the bound only guards against a garbled one.
    for _ in 0..64 {
        let &(parent, path) = processes.get(&current)?;
        if parent <= 1 {
            return outermost_app(path).map(str::to_owned);
        }
        current = parent;
    }
    None
}

fn outermost_app(path: &str) -> Option<&str> {
    let end = path.find(".app/").map(|index| index + ".app".len()).or_else(|| path.ends_with(".app").then_some(path.len()))?;
    Some(&path[..end])
}

/// The process chain above this hook, read from `/proc`, so the common path still spawns nothing.
/// Stops at init, or after 16 levels.
fn ancestor_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    let mut current = std::os::unix::process::parent_id();
    while current > 1 && pids.len() < 16 {
        pids.push(current);
        let Some(parent) = std::fs::read_to_string(format!("/proc/{current}/stat"))
            .ok()
            .and_then(|stat| parent_from_stat(&stat))
        else {
            break;
        };
        current = parent;
    }
    pids
}

/// The parent pid in `/proc/<pid>/stat`. The command name in parentheses may hold spaces and
/// parentheses itself, so fields are counted after the last `)`.
fn parent_from_stat(stat: &str) -> Option<u32> {
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn from_bundle_id(found: Option<&str>, own_bundle_id: &str) -> Surface {
    match found {
        Some(id) if id == own_bundle_id => Surface::App,
        Some(id) if !id.is_empty() => Surface::Host(id.to_owned(), None),
        _ => Surface::Headless,
    }
}

pub fn write_into(payload: &mut Map<String, Value>, surface: &Surface) {
    let name = match surface {
        Surface::App => "app",
        Surface::Host(bundle_id, tab) => {
            payload.insert("host_bundle_id".to_owned(), Value::String(bundle_id.clone()));
            match tab {
                Some(Tab::Ttys(ttys)) => {
                    payload.insert("host_ttys".to_owned(), Value::from(ttys.clone()));
                }
                Some(Tab::Tmux { socket, pane }) => {
                    payload.insert("tmux_socket".to_owned(), Value::String(socket.clone()));
                    payload.insert("tmux_pane".to_owned(), Value::String(pane.clone()));
                }
                None => {}
            }
            "host"
        }
        Surface::Headless => "headless",
        Surface::Window(pids) => {
            payload.insert("host_pids".to_owned(), Value::from(pids.clone()));
            "window"
        }
    };
    payload.insert("surface".to_owned(), Value::String(name.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_pid_survives_odd_command_names() {
        assert_eq!(parent_from_stat("3620 (claude) S 3534 3620 3534 34816"), Some(3534));
        assert_eq!(parent_from_stat("42 (a) b (c)) R 7 42 42 0"), Some(7));
        assert_eq!(parent_from_stat("garbage"), None);
    }

    #[test]
    fn a_tmux_pane_is_reported_by_socket_and_pane() {
        assert_eq!(
            tmux_tab("/private/tmp/tmux-502/default,84233,0", "%3"),
            Some(Tab::Tmux { socket: "/private/tmp/tmux-502/default".to_owned(), pane: "%3".to_owned() })
        );
        assert_eq!(
            tmux_tab("/tmp/odd,name/default,1,0", "%12"),
            Some(Tab::Tmux { socket: "/tmp/odd,name/default".to_owned(), pane: "%12".to_owned() })
        );
        for (tmux, pane) in [("relative,1,0", "%3"), ("/tmp/default,1,0", "3"), ("/tmp/default,1,0", "%3;x"), ("garbage", "%3")] {
            assert_eq!(tmux_tab(tmux, pane), None, "{tmux} {pane}");
        }
    }

    #[test]
    fn a_tmux_pane_goes_into_the_payload_instead_of_a_tty() {
        let mut payload = Map::new();
        let tab = Tab::Tmux { socket: "/tmp/tmux-502/default".to_owned(), pane: "%3".to_owned() };
        write_into(&mut payload, &Surface::Host("com.mitchellh.ghostty".to_owned(), Some(tab)));
        assert_eq!(payload["tmux_socket"], "/tmp/tmux-502/default");
        assert_eq!(payload["tmux_pane"], "%3");
        assert!(!payload.contains_key("host_ttys"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_tty_matches_what_ps_reports() {
        // Under a terminal ps names the tty; under launchd or CI it prints `??` and there is none.
        let output = Command::new("/bin/ps").args(["-o", "tty=", "-p", &std::process::id().to_string()]).output().unwrap();
        let reported = String::from_utf8(output.stdout).unwrap().trim().to_owned();
        let expected = (reported != "??").then(|| format!("tty{}", reported.trim_start_matches("tty")));
        assert_eq!(controlling_ttys().first(), expected.as_ref());
    }

    #[test]
    fn window_pids_are_reported_for_the_daemon() {
        let mut payload = Map::new();
        write_into(&mut payload, &Surface::Window(vec![3620, 3534]));
        assert_eq!(payload["surface"], "window");
        assert_eq!(payload["host_pids"], serde_json::json!([3620, 3534]));
    }

    #[test]
    fn the_agents_own_app_is_not_a_host() {
        assert_eq!(from_bundle_id(Some("com.openai.codex"), "com.openai.codex"), Surface::App);
    }

    #[test]
    fn any_other_app_is_the_landing_spot() {
        assert_eq!(
            from_bundle_id(Some("com.mitchellh.ghostty"), "com.openai.codex"),
            Surface::Host("com.mitchellh.ghostty".to_owned(), None)
        );
    }

    #[test]
    fn an_unknown_terminal_needs_no_code_change() {
        // Unfamiliar terminals take the same path as known ones: jump to whatever we read.
        assert_eq!(
            from_bundle_id(Some("net.example.SomeNewTerminal"), "com.openai.codex"),
            Surface::Host("net.example.SomeNewTerminal".to_owned(), None)
        );
    }

    #[test]
    fn no_bundle_id_means_nowhere_to_go() {
        // CLIs started over SSH, by launchd or by a daemon all end up here.
        assert_eq!(from_bundle_id(None, "com.openai.codex"), Surface::Headless);
        assert_eq!(from_bundle_id(Some(""), "com.openai.codex"), Surface::Headless);
    }

    /// The chain seen on 2026-09-28: ChatGPT.app runs Codex without `__CFBundleIdentifier`.
    const CHATGPT_CHAIN: &str = "    1     0 /sbin/launchd
83759     1 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT
83982 83759 /Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex
87562 83982 /Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node
87653 87562 /Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl
 3388 87653 /Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex
 4242  3388 /Users/dragon/Library/Application Support/VibeBuddy/bin/vibebuddy-hook
";

    #[test]
    fn a_scrubbed_environment_falls_back_to_the_app_launchd_started() {
        assert_eq!(launched_app(CHATGPT_CHAIN, 4242).as_deref(), Some("/Applications/ChatGPT.app"));
    }

    #[test]
    fn a_nested_helper_bundle_is_not_the_landing_spot() {
        // CodexCLI.app has its own bundle id but no window; only the outer app does.
        assert_eq!(
            outermost_app("/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex"),
            Some("/Applications/ChatGPT.app")
        );
        assert_eq!(outermost_app("/usr/sbin/sshd"), None);
    }

    #[test]
    fn a_terminal_with_spaces_in_its_path_is_found() {
        let table = "  1 0 /sbin/launchd
 10 1 /Applications/Some Terminal.app/Contents/MacOS/Some Terminal
 11 10 /bin/zsh
 12 11 /opt/homebrew/bin/codex
 13 12 /Users/me/Library/Application Support/VibeBuddy/bin/vibebuddy-hook
";
        assert_eq!(launched_app(table, 13).as_deref(), Some("/Applications/Some Terminal.app"));
    }

    #[test]
    fn ssh_and_unknown_processes_stay_headless() {
        let table = "  1 0 /sbin/launchd
 20 1 /usr/sbin/sshd
 21 20 /usr/sbin/sshd
 22 21 -zsh
 23 22 /opt/homebrew/bin/codex
";
        assert_eq!(launched_app(table, 23), None);
        assert_eq!(launched_app(table, 999), None);
    }

    #[test]
    fn the_payload_carries_the_landing_spot_only_for_a_host() {
        let mut payload = Map::new();
        write_into(&mut payload, &Surface::Host("com.mitchellh.ghostty".to_owned(), Some(Tab::Ttys(vec!["ttys041".to_owned(), "ttys040".to_owned()]))));
        assert_eq!(payload["surface"], "host");
        assert_eq!(payload["host_bundle_id"], "com.mitchellh.ghostty");
        assert_eq!(payload["host_ttys"], serde_json::json!(["ttys041", "ttys040"]));

        let mut payload = Map::new();
        write_into(&mut payload, &Surface::App);
        assert_eq!(payload["surface"], "app");
        assert!(!payload.contains_key("host_bundle_id"));
    }
}
