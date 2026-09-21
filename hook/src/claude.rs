//! Claude Code 的 Hook：只转发会话与回合标识、事件名、工作目录和子 Agent
//! 身份；`Stop` 时判定是否在等回答。子 Agent 的最后一段是写给父会话的报告，
//! 不是向用户提问，因此只判定主会话。

use serde_json::{Map, Value};

use crate::filter::requires_user_input;
use crate::surface::{self, Surface};

pub const ENDPOINT: &str = "http://127.0.0.1:7331/v1/claude-hooks";
/// Claude App 的 bundle id：`__CFBundleIdentifier` 等于它才可能是 App 会话。
const BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// App 起的 Code 会话才有这个环境变量，值就是桌面会话 id。
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

/// Claude App 的内嵌终端面板，`__CFBundleIdentifier` 同样是 Claude App，但它
/// 是终端场景：只有 App 自己起的 Code 会话才带 `CLAUDE_CODE_HOST_SESSION_ID`。
/// 缺它就把 Claude App 当宿主——K2 把它拉到前台，人就在那个面板里。
fn resolve_surface(detected: Surface, desktop_session: Option<&str>) -> Surface {
    match (detected, desktop_session) {
        (Surface::App, None) => Surface::Host(BUNDLE_ID.to_owned()),
        (detected, _) => detected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 运行处来自环境变量，测试进程里取决于测试跑在哪，与被测的过滤无关。
    /// 判定本身由 resolve_surface 与 surface 模块的纯函数测试覆盖。
    fn without_surface(mut payload: Map<String, Value>) -> Map<String, Value> {
        for key in ["surface", "host_bundle_id", "desktop_session_id"] {
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
        .expect("应有载荷");
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
    fn an_app_session_carries_the_desktop_id_it_was_told() {
        assert_eq!(resolve_surface(Surface::App, Some("local_abc")), Surface::App);
    }

    #[test]
    fn the_apps_own_terminal_panel_is_a_host_not_a_code_session() {
        // 面板里的 CLI 继承了 Claude App 的 bundle id，但没有桌面会话 id。
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
        .expect("应有载荷");
        assert!(!payload.contains_key("response_kind"));
    }
}
