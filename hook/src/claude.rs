//! Claude Code 的 Hook：只转发会话与回合标识、事件名、工作目录和子 Agent
//! 身份；`Stop` 时判定是否在等回答。子 Agent 的最后一段是写给父会话的报告，
//! 不是向用户提问，因此只判定主会话。

use serde_json::{Map, Value};

use crate::filter::requires_user_input;

pub const ENDPOINT: &str = "http://127.0.0.1:7331/v1/claude-hooks";
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
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        .expect("应有载荷");
        assert_eq!(
            Value::Object(payload),
            serde_json::json!({"session_id": "s1", "prompt_id": "p1", "hook_event_name": "UserPromptSubmit", "cwd": "/work/vibe-buddy"})
        );
    }

    #[test]
    fn subagent_identity_is_preserved() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "SubagentStart", "agent_id": "a1", "agent_type": "Explore",
        }))
        .expect("应有载荷");
        assert_eq!(payload["agent_id"], "a1");
        assert_eq!(payload["agent_type"], "Explore");
    }

    #[test]
    fn a_question_becomes_a_derived_flag_without_the_message() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "Stop", "last_assistant_message": "我该用方案 A 还是方案 B？",
        }))
        .expect("应有载荷");
        assert_eq!(payload["response_kind"], "input_required");
        assert!(!payload.contains_key("last_assistant_message"));
    }

    #[test]
    fn a_subagent_report_is_never_classified_as_waiting() {
        let payload = sanitized_payload(&serde_json::json!({
            "session_id": "s1", "prompt_id": "p1", "hook_event_name": "SubagentStop", "agent_id": "a1", "last_assistant_message": "要继续深入排查吗？",
        }))
        .expect("应有载荷");
        assert!(!payload.contains_key("response_kind"));
    }
}
