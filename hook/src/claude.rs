//! Claude Code's hook: forwards only session and turn ids, the event name, the working directory and the subagent
//! identity; on `Stop` it decides whether an answer is awaited. A subagent's last message is a report to the parent session,
//! not a question for the user, so only the main session is judged.

use serde_json::{Map, Value};

use crate::filter::requires_user_input;
use crate::surface::{self, Surface};

pub const ENDPOINT: &str = "http://127.0.0.1:7331/v1/claude-hooks";
/// Claude App's bundle id: only when `__CFBundleIdentifier` equals it can this be an app session.
const BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// Only Code sessions started by the app have this variable; its value is the desktop session id.
const DESKTOP_SESSION_ENV: &str = "CLAUDE_CODE_HOST_SESSION_ID";
const ALLOWED_FIELDS: [&str; 6] = ["session_id", "prompt_id", "hook_event_name", "cwd", "agent_id", "agent_type"];

pub fn sanitized_payload(source: &Value) -> Option<Map<String, Value>> {
    let source = source.as_object()?;
    let mut payload = Map::new();
    for key in ALLOWED_FIELDS {
        if let Some(value) = source.get(key).and_then(Value::as_str) {
            payload.insert(key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if !payload.contains_key("session_id") || !payload.contains_key("hook_event_name") {
        return None;
    }
    if payload.get("hook_event_name").and_then(Value::as_str) == Some("Stop")
        && requires_user_input(source.get("last_assistant_message").and_then(Value::as_str))
    {
        payload.insert("response_kind".to_owned(), Value::String("input_required".to_owned()));
    }
    let desktop_session = std::env::var(DESKTOP_SESSION_ENV).ok().filter(|value| !value.is_empty());
    let surface = resolve_surface(surface::detect(BUNDLE_ID), desktop_session.as_deref());
    if surface == Surface::App && let Some(id) = desktop_session {
        payload.insert("desktop_session_id".to_owned(), Value::String(id));
    }
    surface::write_into(&mut payload, &surface);
    Some(payload)
}

/// Claude App's embedded terminal panel also has Claude App as `__CFBundleIdentifier`, but it
/// is a terminal case: only Code sessions the app starts itself carry `CLAUDE_CODE_HOST_SESSION_ID`.
/// Without it, treat Claude App as the host: K2 brings it to the front, and the user is in that panel.
fn resolve_surface(detected: Surface, desktop_session: Option<&str>) -> Surface {
    match (detected, desktop_session) {
        (Surface::App, None) => Surface::Host(BUNDLE_ID.to_owned()),
        (detected, _) => detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The surface comes from environment variables, so in a test process it depends on where the test runs, unrelated to the filter under test.
    /// The decision itself is covered by resolve_surface and the pure-function tests in the surface module.
    fn without_surface(mut payload: Map<String, Value>) -> Map<String, Value> {
        for key in ["surface", "host_bundle_id", "host_pids", "desktop_session_id"] {
            payload.remove(key);
        }
        payload
    }

    #[test]
    fn prompt_and_transcript_never_leave_the_machine() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1",
            "prompt_id": "p1",
            "hook_event_name": "UserPromptSubmit",
            "cwd": "/work/vibe-buddy",
            "prompt": "请把我的密钥改成 sk-secret",
            "transcript_path": "/Users/someone/.claude/projects/x.jsonl",
            "session_title": "内部项目代号",
        }))
        .expect("should have a payload");
        assert_eq!(
            Value::Object(without_surface(payload)),
            serde_json::json!({"session_id": "s1", "prompt_id": "p1", "hook_event_name": "UserPromptSubmit", "cwd": "/work/vibe-buddy"})
        );
    }

    #[test]
    fn subagent_identity_is_preserved() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "SubagentStart", "agent_id": "a1", "agent_type": "Explore",
        }))
        .expect("should have a payload");
        assert_eq!(payload["agent_id"], "a1");
        assert_eq!(payload["agent_type"], "Explore");
    }

    #[test]
    fn a_question_becomes_a_derived_flag_without_the_message() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "Stop", "last_assistant_message": "我该用方案 A 还是方案 B？",
        }))
        .expect("should have a payload");
        assert_eq!(payload["response_kind"], "input_required");
        assert!(!payload.contains_key("last_assistant_message"));
    }

    #[test]
    fn an_app_session_carries_the_desktop_id_it_was_told() {
        assert_eq!(resolve_surface(Surface::App, Some("local_abc")), Surface::App);
    }

    #[test]
    fn the_apps_own_terminal_panel_is_a_host_not_a_code_session() {
        // The CLI in the panel inherits Claude App's bundle id but has no desktop session id.
        assert_eq!(
            resolve_surface(Surface::App, None),
            Surface::Host(BUNDLE_ID.to_owned())
        );
    }

    #[test]
    fn a_terminal_session_is_left_alone() {
        let ghostty = Surface::Host("com.mitchellh.ghostty".to_owned());
        assert_eq!(resolve_surface(ghostty, None), Surface::Host("com.mitchellh.ghostty".to_owned()));
        assert_eq!(resolve_surface(Surface::Headless, None), Surface::Headless);
    }

    #[test]
    fn a_subagent_report_is_never_classified_as_waiting() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "SubagentStop", "agent_id": "a1", "last_assistant_message": "要继续深入排查吗？",
        }))
        .expect("should have a payload");
        assert!(!payload.contains_key("response_kind"));
    }
}
