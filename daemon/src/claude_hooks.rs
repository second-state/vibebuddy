//! Claude Code 生命周期事件到通用活动模型的映射。
//!
//! 与 Codex Adapter 共用同一个聚合器；差别只在事件名与活动身份的合成方式。

use beacon_protocol::Event;
use serde::Deserialize;

use crate::activity::{ActivityId, ActivityStatus, ActivityTracker, project_title};

/// 任务卡上区分 Agent 的前缀。
const PREFIX: &str = "CC:";
/// 工作目录不可用时的任务卡标题。
const FALLBACK_TITLE: &str = "CLAUDE";

#[derive(Debug, Deserialize)]
pub struct ClaudeHook {
    pub session_id: String,
    #[serde(default)]
    pub prompt_id: Option<String>,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub response_kind: Option<String>,
}

pub fn apply(tracker: &mut ActivityTracker, hook: ClaudeHook) -> Option<Event> {
    let id = activity_id(&hook);
    let title = project_title(PREFIX, hook.cwd.as_deref(), FALLBACK_TITLE);
    match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => {
            tracker.clear_session(&hook.session_id);
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "PermissionRequest" => tracker.require_input(&id, &title),
        "PostToolUse" | "SubagentStart" => tracker.observe(&id, &title, ActivityStatus::Working),
        "Stop" => {
            if hook.response_kind.as_deref() == Some("input_required") {
                return tracker.require_input(&id, &title);
            }
            tracker.finish(&id, &title)
        }
        "SubagentStop" => tracker.finish(&id, &title),
        // 回合因 API 错误结束：既不是成功也不是任务失败。
        "StopFailure" => tracker.discard(&id, "STOPPED"),
        "SessionEnd" => tracker.discard_session(&hook.session_id, "ALL QUIET"),
        _ => None,
    }
}

/// Claude Code 的后台 agent 共享父会话的 `session_id` 与 `prompt_id`，
/// 因此必须把 `agent_id` 并入身份，否则并行的子 agent 会互相覆盖。
fn activity_id(hook: &ClaudeHook) -> ActivityId {
    let mut key = hook.session_id.clone();
    if let Some(prompt_id) = hook.prompt_id.as_deref() {
        key.push(':');
        key.push_str(prompt_id);
    }
    if let Some(agent_id) = hook.agent_id.as_deref() {
        key.push(':');
        key.push_str(agent_id);
    }
    ActivityId {
        session_id: hook.session_id.clone(),
        key,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn hook(name: &str, cwd: &str) -> ClaudeHook {
        ClaudeHook {
            session_id: "session-a".to_owned(),
            prompt_id: Some("turn-a".to_owned()),
            hook_event_name: name.to_owned(),
            cwd: Some(cwd.to_owned()),
            agent_id: None,
            response_kind: None,
        }
    }

    fn subagent_hook(name: &str, agent_id: &str) -> ClaudeHook {
        ClaudeHook {
            agent_id: Some(agent_id.to_owned()),
            ..hook(name, "/work/agent-beacon")
        }
    }

    #[test]
    fn maps_claude_lifecycle_without_prompt_content() {
        let mut tracker = ActivityTracker::default();

        let working = apply(&mut tracker, hook("UserPromptSubmit", "/work/agent-beacon"))
            .expect("开始事件应可见");
        assert_eq!(working.event, "task.start");
        assert_eq!(working.title.as_deref(), Some("CC:AGENT-BEACON"));

        let waiting = apply(
            &mut tracker,
            hook("PermissionRequest", "/work/agent-beacon"),
        )
        .expect("权限请求应可见");
        assert_eq!(waiting.event, "agent.input_required");

        let done = apply(&mut tracker, hook("Stop", "/work/agent-beacon")).expect("停止事件应可见");
        assert_eq!(done.event, "task.done");
    }

    #[test]
    fn parallel_subagents_are_separate_activities() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("UserPromptSubmit", "/work/agent-beacon"));
        apply(&mut tracker, subagent_hook("SubagentStart", "agent-1"));
        let two = apply(&mut tracker, subagent_hook("SubagentStart", "agent-2"))
            .expect("第二个子 agent 应刷新卡片栈");

        let tasks = two.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 3, "父会话与两个子 agent 应各占一张卡");

        let first = apply(&mut tracker, subagent_hook("SubagentStop", "agent-1"))
            .expect("子 agent 结束应产生事件");
        assert_eq!(
            first.extra.get("announcement").and_then(|v| v.as_str()),
            Some("done"),
            "一个子 agent 结束不应吞掉播报"
        );
        let remaining = first.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn stop_that_waits_for_a_reply_requests_input_instead_of_reporting_done() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("UserPromptSubmit", "/work/agent-beacon"));
        let stop: ClaudeHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "prompt_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/agent-beacon",
            "response_kind": "input_required"
        }))
        .expect("等待回答的 Stop 载荷应可解析");

        let waiting = apply(&mut tracker, stop).expect("等待回答应产生可见事件");
        assert_eq!(waiting.event, "agent.input_required");
        assert!(!waiting.extra.contains_key("announcement"));
    }

    #[test]
    fn a_new_turn_replaces_the_previous_one() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("UserPromptSubmit", "/work/alpha"));
        let next = ClaudeHook {
            prompt_id: Some("turn-b".to_owned()),
            ..hook("UserPromptSubmit", "/work/beta")
        };
        let resumed = apply(&mut tracker, next).expect("新 turn 应可见");

        let tasks = resumed.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 1, "同一会话的上一个 turn 应被清除");
        assert_eq!(tasks[0]["title"], "CC:BETA");
    }

    #[test]
    fn api_failure_does_not_report_success() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("UserPromptSubmit", "/work/alpha"));

        let stopped = apply(&mut tracker, hook("StopFailure", "/work/alpha"))
            .expect("API 错误应回到空闲状态");
        assert_eq!(stopped.event, "agent.idle");
        assert_eq!(stopped.title.as_deref(), Some("STOPPED"));
    }

    #[test]
    fn both_agents_share_one_card_stack() {
        use crate::codex_hooks::{self, CodexHook};

        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("UserPromptSubmit", "/work/agent-beacon"));
        let codex = CodexHook {
            session_id: "codex-session".to_owned(),
            turn_id: Some("codex-turn".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: Some("/work/agent-beacon".to_owned()),
            response_kind: None,
        };
        let mixed = codex_hooks::apply(&mut tracker, codex).expect("另一个 Agent 应刷新卡片栈");

        let tasks = mixed.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 2, "同一目录下的两个 Agent 应各占一张卡");
        assert_eq!(tasks[0]["title"], "CX:AGENT-BEACON");
        assert_eq!(tasks[1]["title"], "CC:AGENT-BEACON");
    }
}
