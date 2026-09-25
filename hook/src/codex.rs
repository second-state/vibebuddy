//! Codex's hook: forwards only session and turn ids, the event name and the working directory; subagent
//! lifecycle sessions map to the parent thread the desktop app can open; on `Stop` it decides whether an answer is awaited.

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

use crate::filter::requires_user_input;
use crate::surface;

pub const ENDPOINT: &str = "http://127.0.0.1:7331/v1/codex-hooks";
/// Codex App's bundle id. It ships inside ChatGPT.app, and the `codex:` scheme is claimed by that
/// bundle too (checked with lsregister on 2026-09-21).
const BUNDLE_ID: &str = "com.openai.codex";
const ALLOWED_FIELDS: [&str; 4] = ["session_id", "turn_id", "hook_event_name", "cwd"];

/// session_meta from the transcript's first line; treated as absent if it can't be read.
fn transcript_meta(path: &str) -> Option<Map<String, Value>> {
    let file = std::fs::File::open(path).ok()?;
    let mut first_line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(file), &mut first_line).ok()?;
    let value: Value = serde_json::from_str(&first_line).ok()?;
    value.get("payload")?.as_object().cloned()
}

/// Maps a subagent's lifecycle session to the parent session the desktop app can open.
pub fn navigable_thread_id(source: &Map<String, Value>) -> Option<String> {
    let session_id = source.get("session_id")?.as_str()?.to_owned();
    let Some(transcript_path) = source.get("transcript_path").and_then(Value::as_str) else {
        return Some(session_id);
    };
    let Some(payload) = transcript_meta(transcript_path) else {
        return Some(session_id);
    };
    let own_thread_id = payload
        .get("id")
        .and_then(Value::as_str)
        .map_or(session_id, str::to_owned);
    if payload.get("thread_source").and_then(Value::as_str) != Some("subagent") {
        return Some(own_thread_id);
    }
    payload
        .get("source")
        .and_then(|origin| origin.get("subagent"))
        .and_then(|subagent| subagent.get("thread_spawn"))
        .and_then(|spawn| spawn.get("parent_thread_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(Some(own_thread_id))
}

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
    if let Some(thread_id) = navigable_thread_id(source) {
        payload.insert("thread_id".to_owned(), Value::String(thread_id));
    }
    if payload.get("hook_event_name").and_then(Value::as_str) == Some("Stop")
        && requires_user_input(source.get("last_assistant_message").and_then(Value::as_str))
    {
        payload.insert("response_kind".to_owned(), Value::String("input_required".to_owned()));
    }
    surface::write_into(&mut payload, &surface::detect(BUNDLE_ID));
    Some(payload)
}

/// Logs one diagnostic line locally: only identity, event name and directory, never the prompt.
/// Codex fires hooks for background sessions too; when something goes wrong we need to see what it actually received.
pub fn trace(source: &Value, payload: &Map<String, Value>, log_dir: &Path) {
    let source = source.as_object().cloned().unwrap_or_default();
    let transcript = source.get("transcript_path").and_then(Value::as_str);
    let thread_source = match transcript {
        Some(path) => transcript_meta(path)
            .and_then(|meta| meta.get("thread_source").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "unreadable".to_owned()),
        None => "-".to_owned(),
    };
    let short = |key: &str, fallback: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(|value| value.chars().take(13).collect::<String>())
            .unwrap_or_else(|| fallback.to_owned())
    };
    let mut keys: Vec<&str> = source
        .keys()
        .map(String::as_str)
        .filter(|key| *key != "last_assistant_message")
        .collect();
    keys.sort_unstable();
    let line = format!(
        "{} {} session={} thread={} cwd={} transcript={} thread_source={} keys={}\n",
        chrono::Local::now().format("%m-%d %H:%M:%S"),
        payload.get("hook_event_name").and_then(Value::as_str).unwrap_or("?"),
        short("session_id", "?"),
        short("thread_id", "-"),
        payload.get("cwd").and_then(Value::as_str).unwrap_or("-"),
        if transcript.is_some() { "yes" } else { "no" },
        thread_source,
        keys.join(","),
    );
    // Diagnostics must not affect the main flow: if the write fails, let it go.
    let _ = std::fs::create_dir_all(log_dir);
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(log_dir.join("codex-hooks.log"))
    {
        let _ = log.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_transcript(name: &str, meta: Value) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vibebuddy-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录");
        let path = dir.join(name);
        std::fs::write(&path, format!("{}\n", serde_json::json!({"type": "session_meta", "payload": meta})))
            .expect("写 transcript");
        path
    }

    #[test]
    fn subagent_activity_opens_its_parent_desktop_thread() {
        let transcript = temp_transcript(
            "rollout-child.jsonl",
            serde_json::json!({
                "id": "child-session",
                "thread_source": "subagent",
                "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent-thread", "depth": 1}}}
            }),
        );
        let source = serde_json::json!({
            "session_id": "child-session",
            "turn_id": "child-turn",
            "hook_event_name": "PostToolUse",
            "cwd": "/work/memories",
            "transcript_path": transcript.to_string_lossy(),
        });
        let payload = sanitized_payload(&source).expect("应有载荷");
        assert_eq!(payload["thread_id"], "parent-thread");
        assert!(!payload.contains_key("transcript_path"));
    }

    #[test]
    fn user_activity_opens_its_own_desktop_thread() {
        let transcript = temp_transcript(
            "rollout-user.jsonl",
            serde_json::json!({"id": "user-session", "thread_source": "user", "source": "vscode"}),
        );
        let source = serde_json::json!({
            "session_id": "user-session",
            "turn_id": "user-turn",
            "hook_event_name": "PostToolUse",
            "cwd": "/work/vibe-buddy",
            "transcript_path": transcript.to_string_lossy(),
        });
        let payload = sanitized_payload(&source).expect("应有载荷");
        assert_eq!(payload["thread_id"], "user-session");
        assert!(!payload.contains_key("transcript_path"));
    }

    #[test]
    fn a_question_is_classified_without_forwarding_the_message() {
        let source = serde_json::json!({
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/vibe-buddy",
            "last_assistant_message": "第一项推荐使用 GitHub Issues。\n\n是否采用 GitHub Issues？直接回复“是”即可。",
        });
        let payload = sanitized_payload(&source).expect("应有载荷");
        assert_eq!(payload["response_kind"], "input_required");
        assert!(!payload.contains_key("last_assistant_message"));
    }

    #[test]
    fn a_completed_response_is_not_input_required() {
        let source = serde_json::json!({
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/vibe-buddy",
            "last_assistant_message": "修复已完成，全量测试通过。",
        });
        let payload = sanitized_payload(&source).expect("应有载荷");
        assert!(!payload.contains_key("response_kind"));
    }

    #[test]
    fn an_optional_follow_up_offer_is_not_required_input() {
        for message in ["如果你还需要调整配色，可以告诉我。", "修复完成。If you need anything else, let me know."] {
            let source = serde_json::json!({
                "session_id": "session-a",
                "turn_id": "turn-a",
                "hook_event_name": "Stop",
                "cwd": "/work/vibe-buddy",
                "last_assistant_message": message,
            });
            let payload = sanitized_payload(&source).expect("应有载荷");
            assert!(!payload.contains_key("response_kind"), "{message}");
        }
    }

    #[test]
    fn a_payload_without_identity_is_dropped() {
        assert!(sanitized_payload(&serde_json::json!({"hook_event_name": "Stop"})).is_none());
        assert!(sanitized_payload(&serde_json::json!("not an object")).is_none());
    }
}
