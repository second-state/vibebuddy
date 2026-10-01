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

use std::collections::HashMap;
use std::process::Command;

use serde_json::{Map, Value};

const BUNDLE_ID: &str = "__CFBundleIdentifier";

#[derive(Debug, PartialEq, Eq)]
pub enum Surface {
    /// Runs in the agent's own desktop app; K2 uses that agent's deeplink.
    App,
    /// Runs inside another app, and this bundle id is where K2 goes. Unknown apps are all treated
    /// as hosts, so even unfamiliar terminals are reached correctly.
    Host(String),
    /// No host app: sessions started over SSH, by a daemon, or by launchd. K2 has nowhere to go.
    Headless,
}

pub fn detect(own_bundle_id: &str) -> Surface {
    let found = std::env::var(BUNDLE_ID)
        .ok()
        .filter(|id| !id.is_empty())
        .or_else(launched_app_bundle_id);
    from_bundle_id(found.as_deref(), own_bundle_id)
}

/// The bundle id of the app launchd started at the top of this process chain, if it is an app.
/// Only runs when `__CFBundleIdentifier` is missing, so the common path spawns nothing.
fn launched_app_bundle_id() -> Option<String> {
    // App bundles exist only on macOS; elsewhere this would spawn `ps` on every hook for nothing.
    if !cfg!(target_os = "macos") {
        return None;
    }
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

fn from_bundle_id(found: Option<&str>, own_bundle_id: &str) -> Surface {
    match found {
        Some(id) if id == own_bundle_id => Surface::App,
        Some(id) if !id.is_empty() => Surface::Host(id.to_owned()),
        _ => Surface::Headless,
    }
}

pub fn write_into(payload: &mut Map<String, Value>, surface: &Surface) {
    let name = match surface {
        Surface::App => "app",
        Surface::Host(bundle_id) => {
            payload.insert("host_bundle_id".to_owned(), Value::String(bundle_id.clone()));
            "host"
        }
        Surface::Headless => "headless",
    };
    payload.insert("surface".to_owned(), Value::String(name.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agents_own_app_is_not_a_host() {
        assert_eq!(from_bundle_id(Some("com.openai.codex"), "com.openai.codex"), Surface::App);
    }

    #[test]
    fn any_other_app_is_the_landing_spot() {
        assert_eq!(
            from_bundle_id(Some("com.mitchellh.ghostty"), "com.openai.codex"),
            Surface::Host("com.mitchellh.ghostty".to_owned())
        );
    }

    #[test]
    fn an_unknown_terminal_needs_no_code_change() {
        // Unfamiliar terminals take the same path as known ones: jump to whatever we read.
        assert_eq!(
            from_bundle_id(Some("net.example.SomeNewTerminal"), "com.openai.codex"),
            Surface::Host("net.example.SomeNewTerminal".to_owned())
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
        write_into(&mut payload, &Surface::Host("com.mitchellh.ghostty".to_owned()));
        assert_eq!(payload["surface"], "host");
        assert_eq!(payload["host_bundle_id"], "com.mitchellh.ghostty");

        let mut payload = Map::new();
        write_into(&mut payload, &Surface::App);
        assert_eq!(payload["surface"], "app");
        assert!(!payload.contains_key("host_bundle_id"));
    }
}
