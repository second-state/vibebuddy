//! Decides where the agent process runs: in its own desktop app, inside another app (terminal,
//! editor), or with no host at all.
//!
//! The only evidence is `__CFBundleIdentifier`, injected by LaunchServices when it launches an app and inherited down the
//! process chain to the hook. `TERM_PROGRAM` isn't used: it would need a table mapping terminal names to bundle ids,
//! which breaks as soon as the user switches to an unfamiliar terminal. Nor the tty: agents run tool commands in
//! non-interactive child processes that report not a tty even when the host is a terminal (verified by running
//! codex in Ghostty on 2026-09-21).

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
    from_bundle_id(std::env::var(BUNDLE_ID).ok().as_deref(), own_bundle_id)
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
