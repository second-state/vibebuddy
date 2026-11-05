//! Agents with no desktop app of their own to jump back into: OpenCode, GitHub Copilot CLI, Pi and ZCode. Each speaks its own
//! events; the hook turns them into five that mean the same for every such agent (`working`, `needs_input`,
//! `done`, `stopped`, `session_end`), so the daemon needs one adapter for all of them, not one per agent.
//!
//! No config is merged into the user's own: VibeBuddy owns a whole file in each agent's directory, the
//! OpenCode plugin, the Copilot hooks file or the Pi extension, so connecting writes it and removing deletes it. Their contents are
//! ZCode registers an isolated local plugin through `zcode.rs`. Other owned files are
//! made here and nowhere else; the Mac app asks this binary for them (`vibebuddy-hook agent-file`).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::filter::requires_user_input;
use crate::surface;

pub const ENDPOINT: &str = "http://127.0.0.1:7331/v1/agent-hooks";
const EVENTS: [&str; 5] = ["working", "needs_input", "done", "stopped", "session_end"];
const OPENCODE_PLUGIN: &str = include_str!("opencode-plugin.js");
const PI_EXTENSION: &str = include_str!("pi-extension.js");
/// The Copilot events subscribed to, and the normalized event each one becomes.
const COPILOT_EVENTS: [(&str, &str); 6] = [
    ("userPromptSubmitted", "working"),
    ("postToolUse", "working"),
    ("notification", "needs_input"),
    ("agentStop", "done"),
    ("errorOccurred", "stopped"),
    ("sessionEnd", "session_end"),
];
/// Copilot's notifications that mean it is waiting on the user; the rest are about background work.
const COPILOT_WAITING: &str = "permission_prompt|elicitation_dialog";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    OpenCode,
    Copilot,
    Pi,
    Zcode,
}

impl Agent {
    pub const ALL: [Agent; 4] = [Agent::OpenCode, Agent::Copilot, Agent::Pi, Agent::Zcode];

    pub fn argument(self) -> &'static str {
        match self {
            Agent::OpenCode => "opencode",
            Agent::Copilot => "copilot",
            Agent::Pi => "pi",
            Agent::Zcode => "zcode",
        }
    }

    pub fn from_argument(argument: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|agent| agent.argument() == argument)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Agent::OpenCode => "OpenCode",
            Agent::Copilot => "GitHub Copilot CLI",
            Agent::Pi => "Pi",
            Agent::Zcode => "ZCode",
        }
    }

    /// The agent's own directory, which exists once it has run here. Copilot honours `COPILOT_HOME`, Pi
    /// `PI_CODING_AGENT_DIR`.
    pub fn dir(self, home: &Path) -> PathBuf {
        match self {
            Agent::OpenCode => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode"),
            Agent::Copilot => std::env::var_os("COPILOT_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".copilot")),
            Agent::Zcode => home.join(".zcode/cli"),
            Agent::Pi => std::env::var_os("PI_CODING_AGENT_DIR")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".pi/agent")),
        }
    }

    /// The file VibeBuddy owns in that directory.
    pub fn file(self, home: &Path) -> PathBuf {
        match self {
            Agent::OpenCode => self.dir(home).join("plugins/vibebuddy.js"),
            Agent::Copilot => self.dir(home).join("hooks/vibebuddy.json"),
            Agent::Pi => self.dir(home).join("extensions/vibebuddy.js"),
            Agent::Zcode => self.dir(home).join("config.json"),
        }
    }

    /// What that file holds, pointing at the hook binary at `hook`.
    pub fn file_contents(self, hook: &str) -> String {
        let hook_literal = Value::String(hook.to_owned()).to_string();
        match self {
            Agent::OpenCode => OPENCODE_PLUGIN.replace("__HOOK__", &hook_literal),
            Agent::Zcode => crate::zcode::manifest(hook).to_string(),
            Agent::Pi => PI_EXTENSION.replace("__HOOK__", &hook_literal),
            Agent::Copilot => {
                let hooks: Map<String, Value> = COPILOT_EVENTS
                    .iter()
                    .map(|(event, _)| {
                        // `exec` runs the hook without a shell, so a path with spaces needs no quoting.
                        let mut entry = json!({ "type": "command", "exec": hook, "args": ["copilot", event], "timeoutSec": 5 });
                        if *event == "notification" {
                            entry["matcher"] = json!(COPILOT_WAITING);
                        }
                        ((*event).to_owned(), json!([entry]))
                    })
                    .collect();
                let mut text =
                    serde_json::to_string_pretty(&json!({ "version": 1, "hooks": hooks }))
                        .unwrap_or_default();
                text.push('\n');
                text
            }
        }
    }
}

/// The OpenCode plugin and the Pi extension already speak the normalized events; only the fields that may leave
/// are kept.
pub fn plugin_payload(agent: Agent, source: &Value) -> Option<Map<String, Value>> {
    let event = source.get("event")?.as_str()?;
    let reply = source.get("last_assistant_message").and_then(Value::as_str);
    payload(
        agent,
        event,
        source.get("session_id"),
        source.get("turn_id"),
        source.get("cwd"),
        reply,
    )
}

/// Copilot doesn't name the event in camelCase payloads, so the hooks file passes it as an argument.
pub fn copilot_payload(copilot_event: &str, source: &Value) -> Option<Map<String, Value>> {
    let (_, event) = COPILOT_EVENTS
        .iter()
        .find(|(name, _)| *name == copilot_event)?;
    if copilot_event == "notification" {
        let kind = source
            .get("notification_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !COPILOT_WAITING.split('|').any(|waiting| waiting == kind) {
            return None;
        }
    }
    // Copilot hands over no reply text at the end of a turn, only a transcript path; a question left in the
    // reply is announced as done.
    payload(
        Agent::Copilot,
        event,
        source.get("sessionId"),
        None,
        source.get("cwd"),
        None,
    )
}

/// Reduce Kimi's hook input to the fields accepted by the generic agent endpoint.
pub fn kimi_payload(source: &Value) -> Option<Map<String, Value>> {
    let event = match source.get("hook_event_name")?.as_str()? {
        "UserPromptSubmit" | "PostToolUse" => "working",
        "PermissionRequest" => "needs_input",
        "Stop" => "done",
        "StopFailure" | "Interrupt" => "stopped",
        "SessionEnd" => "session_end",
        _ => return None,
    };
    let session = source
        .get("session_id")?
        .as_str()
        .filter(|id| !id.is_empty())?;
    let mut result = Map::new();
    result.insert("agent".into(), Value::from("kimi"));
    result.insert("session_id".into(), Value::from(session));
    result.insert("event".into(), Value::from(event));
    if let Some(cwd) = source
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.is_empty())
    {
        result.insert("cwd".into(), Value::from(cwd));
    }
    surface::write_into(&mut result, &surface::detect(""));
    Some(result)
}

pub fn zcode_payload(source: &Value) -> Option<Map<String, Value>> {
    let event = match source.get("hookEventName").or_else(|| source.get("hook_event_name"))?.as_str()? {
        "UserPromptSubmit" | "PostToolUse" | "PostToolUseFailure" => "working",
        "PermissionRequest" => "needs_input",
        "Stop" => "done",
        _ => return None,
    };
    payload(Agent::Zcode, event, source.get("sessionId").or_else(|| source.get("session_id")), source.get("turnId"), source.get("cwd"), source.get("responseText").or_else(|| source.get("last_assistant_message")).and_then(Value::as_str))
}

fn payload(
    agent: Agent,
    event: &str,
    session_id: Option<&Value>,
    turn_id: Option<&Value>,
    cwd: Option<&Value>,
    reply: Option<&str>,
) -> Option<Map<String, Value>> {
    if !EVENTS.contains(&event) {
        return None;
    }
    let session_id = session_id?.as_str().filter(|id| !id.is_empty())?;
    let mut payload = Map::new();
    payload.insert("agent".to_owned(), Value::from(agent.argument()));
    payload.insert("session_id".to_owned(), Value::from(session_id));
    payload.insert("event".to_owned(), Value::from(event));
    for (key, value) in [("turn_id", turn_id), ("cwd", cwd)] {
        if let Some(value) = value
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            payload.insert(key.to_owned(), Value::from(value));
        }
    }
    // The reply itself stays here; only the verdict goes on, as for Claude Code.
    if event == "done" && requires_user_input(reply) {
        payload.insert("response_kind".to_owned(), Value::from("input_required"));
    }
    // These agents have no app of their own, so whatever app they run in is a host.
    surface::write_into(&mut payload, &surface::detect(""));
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only keys the generic endpoint receives from a Kimi hook: the identity, the event, the directory, and
    /// the host the agent runs in (`surface::write_into`).
    const KIMI_PAYLOAD_KEYS: [&str; 11] = [
        "agent",
        "session_id",
        "event",
        "cwd",
        "surface",
        "host_bundle_id",
        "host_ttys",
        "host_focus_url",
        "tmux_socket",
        "tmux_pane",
        "host_pids",
    ];

    #[test]
    fn kimi_events_are_normalized_without_forwarding_content() {
        // Every field a Kimi hook may carry that isn't allowed through, each holding the same marker.
        const SECRET: &str = "SECRET-NEVER-FORWARDED";
        for (input, expected) in [
            ("UserPromptSubmit", "working"),
            ("PostToolUse", "working"),
            ("PermissionRequest", "needs_input"),
            ("Stop", "done"),
            ("StopFailure", "stopped"),
            ("Interrupt", "stopped"),
            ("SessionEnd", "session_end"),
        ] {
            let source = json!({
                "hook_event_name": input, "session_id": "s1", "cwd": "/work",
                "prompt": SECRET, "tool_name": SECRET, "tool_input": {"command": SECRET},
                "tool_output": SECRET, "tool_response": SECRET, "error": SECRET,
                "error_message": SECRET, "session_title": SECRET, "transcript_path": SECRET,
                "last_assistant_message": SECRET, "message": SECRET, "reason": SECRET,
                "messages": [SECRET], "nested": {"prompt": SECRET}, "stop_hook_active": true,
            });
            let result = kimi_payload(&source).expect("recognized event");
            assert_eq!(result["event"], expected);
            assert_eq!(result["agent"], "kimi");
            assert_eq!(result["session_id"], "s1", "the session is forwarded exactly");
            assert_eq!(result["cwd"], "/work", "the directory is forwarded exactly");
            assert!(
                result.keys().all(|key| KIMI_PAYLOAD_KEYS.contains(&key.as_str())),
                "unexpected keys: {:?}",
                result.keys().collect::<Vec<_>>()
            );
            assert!(!serde_json::to_string(&result).unwrap().contains(SECRET));
        }
    }

    #[test]
    fn kimi_events_without_a_known_name_or_session_are_dropped() {
        assert!(
            kimi_payload(&json!({"hook_event_name": "Notification", "session_id": "s1"})).is_none()
        );
        assert!(kimi_payload(&json!({"hook_event_name": "SubagentStop", "session_id": "s1"})).is_none());
        assert!(kimi_payload(&json!({"hook_event_name": "Stop"})).is_none());
        assert!(kimi_payload(&json!({"hook_event_name": "Stop", "session_id": ""})).is_none());
        assert!(kimi_payload(&json!({"hook_event_name": "Stop", "session_id": 7})).is_none());
        assert!(kimi_payload(&json!({"session_id": "s1"})).is_none());
        let without_directory =
            kimi_payload(&json!({"hook_event_name": "Stop", "session_id": "s1"})).expect("payload");
        assert!(!without_directory.contains_key("cwd"));
    }

    #[test]
    fn an_opencode_event_keeps_only_ids_and_the_directory() {
        let source = json!({
            "event": "working", "session_id": "ses_1", "turn_id": "msg_1", "cwd": "/work/vibe-buddy",
            "prompt": "never forwarded"
        });
        let payload = plugin_payload(Agent::OpenCode, &source).expect("payload");
        assert_eq!(payload["agent"], "opencode");
        assert_eq!(payload["event"], "working");
        assert_eq!(payload["turn_id"], "msg_1");
        assert!(!payload.contains_key("prompt"));
    }

    #[test]
    fn a_reply_ending_in_a_question_is_waiting_and_the_reply_stays_behind() {
        let source = json!({ "event": "done", "session_id": "ses_1", "last_assistant_message": "Which database should I use?" });
        let payload = plugin_payload(Agent::OpenCode, &source).expect("payload");
        assert_eq!(payload["response_kind"], "input_required");
        assert!(!payload.contains_key("last_assistant_message"));

        let done = json!({ "event": "done", "session_id": "ses_1", "last_assistant_message": "All tests pass." });
        assert!(
            !plugin_payload(Agent::OpenCode, &done)
                .expect("payload")
                .contains_key("response_kind")
        );
    }

    #[test]
    fn replies_captured_from_opencode_are_judged_like_claudes() {
        // Two real replies the plugin forwarded from OpenCode 1.16.2 on 2026-10-08.
        let question = "我检测到：询问意图 — 用户要我提出一个单一问题以询问其偏好的编程语言。我的方法：直接提问。\n\n你更喜欢哪种编程语言？";
        let answer = "我检测到简单（trivial）意图 — 用户要求一个单词回复。我的方法：直接回答。\nok";
        let judged = |reply: &str| {
            let source =
                json!({ "event": "done", "session_id": "ses_1", "last_assistant_message": reply });
            plugin_payload(Agent::OpenCode, &source)
                .expect("payload")
                .contains_key("response_kind")
        };
        assert!(judged(question));
        assert!(!judged(answer));
    }

    #[test]
    fn unknown_events_and_missing_sessions_are_dropped() {
        assert!(
            plugin_payload(
                Agent::OpenCode,
                &json!({ "event": "rm -rf", "session_id": "ses_1" })
            )
            .is_none()
        );
        assert!(plugin_payload(Agent::OpenCode, &json!({ "event": "done" })).is_none());
        assert!(copilot_payload("preToolUse", &json!({ "sessionId": "s1" })).is_none());
    }

    #[test]
    fn copilot_events_map_onto_the_shared_ones() {
        let source = json!({ "sessionId": "s1", "cwd": "/work/app", "stopReason": "end_turn" });
        let payload = copilot_payload("agentStop", &source).expect("payload");
        assert_eq!(payload["agent"], "copilot");
        assert_eq!(payload["event"], "done");
        assert_eq!(payload["session_id"], "s1");
        assert_eq!(payload["cwd"], "/work/app");
    }

    #[test]
    fn only_copilot_notifications_that_wait_on_the_user_count() {
        let asking = json!({ "sessionId": "s1", "notification_type": "permission_prompt" });
        assert_eq!(
            copilot_payload("notification", &asking).expect("payload")["event"],
            "needs_input"
        );
        let background = json!({ "sessionId": "s1", "notification_type": "shell_completed" });
        assert!(copilot_payload("notification", &background).is_none());
    }

    #[test]
    fn the_copilot_file_runs_the_hook_without_a_shell_for_every_event() {
        let text = Agent::Copilot
            .file_contents("/Users/me/Library/Application Support/VibeBuddy/bin/vibebuddy-hook");
        let config: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(config["version"], 1);
        for (event, _) in COPILOT_EVENTS {
            let entry = &config["hooks"][event][0];
            assert_eq!(
                entry["exec"],
                "/Users/me/Library/Application Support/VibeBuddy/bin/vibebuddy-hook"
            );
            assert_eq!(entry["args"], json!(["copilot", event]));
        }
        assert_eq!(
            config["hooks"]["notification"][0]["matcher"],
            COPILOT_WAITING
        );
    }

    #[test]
    fn the_plugin_and_the_extension_point_at_the_hook_as_a_string_literal() {
        for agent in [Agent::OpenCode, Agent::Pi] {
            let text = agent.file_contents("/odd \"path\"/vibebuddy-hook");
            assert!(text.contains(r#"const HOOK = "/odd \"path\"/vibebuddy-hook";"#));
            assert!(!text.contains("__HOOK__"));
        }
    }

    #[test]
    fn a_pi_event_is_filed_under_pi() {
        let source = json!({ "event": "done", "session_id": "019a", "turn_id": "1760000000000", "cwd": "/work/app",
            "last_assistant_message": "Done, all tests pass." });
        let payload = plugin_payload(Agent::Pi, &source).expect("payload");
        assert_eq!(payload["agent"], "pi");
        assert_eq!(payload["session_id"], "019a");
        assert!(!payload.contains_key("last_assistant_message"));
        assert!(!payload.contains_key("response_kind"));
    }
}
