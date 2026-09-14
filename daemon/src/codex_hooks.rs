//! Codex 生命周期事件到通用活动模型的映射。
//!
//! 这里只做翻译：任务卡聚合、去重、播报和过期都在 [`crate::activity`]。

use beacon_protocol::Event;
use serde::Deserialize;

use crate::activity::{ActivityId, ActivityStatus, ActivityTracker, project_title};

/// 工作目录不可用时的任务卡标题。
const FALLBACK_TITLE: &str = "CODEX";

#[derive(Debug, Deserialize)]
pub struct CodexHook {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub response_kind: Option<String>,
}

#[derive(Default)]
pub struct CodexActivityTracker {
    activities: ActivityTracker,
}

impl CodexActivityTracker {
    pub fn apply(&mut self, hook: CodexHook) -> Option<Event> {
        let id = activity_id(&hook);
        let title = project_title(hook.cwd.as_deref(), FALLBACK_TITLE);
        match hook.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                self.activities.clear_session(&hook.session_id);
                self.activities
                    .observe(&id, &title, ActivityStatus::Working)
            }
            "PermissionRequest" => self.activities.require_input(&id, &title),
            "PostToolUse" => self
                .activities
                .observe(&id, &title, ActivityStatus::Working),
            "Stop" => {
                if hook.response_kind.as_deref() == Some("input_required") {
                    return self.activities.require_input(&id, &title);
                }
                self.activities.finish(&id, &title)
            }
            "Interrupt" => self.activities.discard(&id, "INTERRUPTED"),
            "SessionEnd" => self
                .activities
                .discard_session(&hook.session_id, "ALL QUIET"),
            _ => None,
        }
    }

    pub fn sweep_expired(&mut self) -> Option<Event> {
        self.activities.sweep_expired()
    }
}

/// Codex 用 `session_id` 与 `turn_id` 合成活动身份。
fn activity_id(hook: &CodexHook) -> ActivityId {
    let key = match hook.turn_id.as_deref() {
        Some(turn_id) => format!("{}:{turn_id}", hook.session_id),
        None => hook.session_id.clone(),
    };
    ActivityId {
        session_id: hook.session_id.clone(),
        key,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn hook(session: &str, name: &str, cwd: &str) -> CodexHook {
        hook_with_turn(session, &format!("{session}-turn"), name, cwd)
    }

    fn hook_with_turn(session: &str, turn: &str, name: &str, cwd: &str) -> CodexHook {
        CodexHook {
            session_id: session.to_owned(),
            turn_id: Some(turn.to_owned()),
            hook_event_name: name.to_owned(),
            cwd: Some(cwd.to_owned()),
            response_kind: None,
        }
    }

    #[test]
    fn maps_codex_lifecycle_without_prompt_content() {
        let mut tracker = CodexActivityTracker::default();

        let working = tracker
            .apply(hook("session-a", "UserPromptSubmit", "/work/agent-beacon"))
            .expect("开始事件应可见");
        assert_eq!(working.event, "task.start");
        assert_eq!(working.title.as_deref(), Some("AGENT-BEACON"));

        let waiting = tracker
            .apply(hook("session-a", "PermissionRequest", "/work/agent-beacon"))
            .expect("审批事件应可见");
        assert_eq!(waiting.event, "agent.input_required");

        let resumed = tracker
            .apply(hook("session-a", "PostToolUse", "/work/agent-beacon"))
            .expect("工具完成后应恢复工作中");
        assert_eq!(resumed.event, "task.start");

        let done = tracker
            .apply(hook("session-a", "Stop", "/work/agent-beacon"))
            .expect("停止事件应可见");
        assert_eq!(done.event, "task.done");
    }

    #[test]
    fn stop_that_waits_for_a_reply_requests_input_instead_of_reporting_done() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("session-a", "UserPromptSubmit", "/work/agent-beacon"));
        let stop: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "session-a-turn",
            "hook_event_name": "Stop",
            "cwd": "/work/agent-beacon",
            "response_kind": "input_required"
        }))
        .expect("等待回答的 Stop 载荷应可解析");

        let waiting = tracker.apply(stop).expect("等待回答应产生可见事件");
        assert_eq!(waiting.event, "agent.input_required");
        assert!(!waiting.extra.contains_key("announcement"));
    }

    #[test]
    fn input_required_has_priority_over_other_work() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("working", "UserPromptSubmit", "/work/alpha"));
        tracker.apply(hook("waiting", "UserPromptSubmit", "/work/beta"));

        let waiting = tracker
            .apply(hook("waiting", "PermissionRequest", "/work/beta"))
            .expect("需要输入应成为可见状态");
        assert_eq!(waiting.event, "agent.input_required");
        assert_eq!(waiting.title.as_deref(), Some("BETA"));

        let fallback = tracker
            .apply(hook("waiting", "Stop", "/work/beta"))
            .expect("高优先级任务结束后应恢复另一个工作任务");
        assert_eq!(fallback.event, "task.start");
        assert_eq!(fallback.title.as_deref(), Some("ALPHA"));
    }

    #[test]
    fn background_refresh_does_not_reannounce_existing_input_request() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("waiting", "UserPromptSubmit", "/work/waiting"));
        let first = tracker
            .apply(hook("waiting", "PermissionRequest", "/work/waiting"))
            .expect("首次等待输入应可见");
        assert!(!first.extra.contains_key("suppress_audio"));

        let refreshed = tracker
            .apply(hook("working", "UserPromptSubmit", "/work/working"))
            .expect("后台任务变化应刷新卡片");
        assert_eq!(refreshed.event, "agent.input_required");
        assert_eq!(refreshed.extra["suppress_audio"], true);
    }

    #[test]
    fn task_cards_are_newest_first_and_limited_to_three() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("one", "UserPromptSubmit", "/work/one"));
        tracker.apply(hook("two", "UserPromptSubmit", "/work/two"));
        tracker.apply(hook("three", "UserPromptSubmit", "/work/three"));
        let latest = tracker
            .apply(hook("four", "UserPromptSubmit", "/work/four"))
            .expect("新任务应刷新卡片栈");

        let tasks = latest.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0]["title"], "FOUR");
        assert_eq!(tasks[1]["title"], "THREE");
        assert_eq!(tasks[2]["title"], "TWO");
    }

    #[test]
    fn background_task_removal_refreshes_the_stack() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("old", "UserPromptSubmit", "/work/old"));
        tracker.apply(hook("new", "UserPromptSubmit", "/work/new"));

        let refreshed = tracker
            .apply(hook("old", "Stop", "/work/old"))
            .expect("后台任务结束也应刷新卡片栈");
        let tasks = refreshed.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "NEW");
    }

    #[test]
    fn duplicate_visible_state_is_suppressed() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("session-a", "UserPromptSubmit", "/work/alpha"));

        assert!(
            tracker
                .apply(hook("session-a", "PostToolUse", "/work/alpha"))
                .is_none()
        );
    }

    #[test]
    fn reply_starts_a_new_turn_and_clears_the_previous_waiting_turn() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook_with_turn(
            "session-a",
            "turn-a",
            "UserPromptSubmit",
            "/work/alpha",
        ));
        let waiting: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/alpha",
            "response_kind": "input_required"
        }))
        .expect("等待回答的 Stop 载荷应可解析");
        assert_eq!(
            tracker.apply(waiting).expect("提问应等待回答").event,
            "agent.input_required"
        );

        let resumed = tracker
            .apply(hook_with_turn(
                "session-a",
                "turn-b",
                "UserPromptSubmit",
                "/work/beta",
            ))
            .expect("用户回答后应开始新 turn");
        assert_eq!(resumed.event, "task.start");
        assert_eq!(resumed.title.as_deref(), Some("BETA"));

        let replay = tracker.apply(hook_with_turn("session-a", "turn-a", "Stop", "/work/alpha"));
        assert!(replay.is_none(), "旧 turn 的重复 Stop 不应改变新 turn");

        let second = tracker
            .apply(hook_with_turn("session-a", "turn-b", "Stop", "/work/beta"))
            .expect("第二个 turn 应产生完成通知");
        assert_eq!(second.event, "task.done");
    }

    #[test]
    fn interrupt_does_not_report_success() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("session-a", "UserPromptSubmit", "/work/alpha"));

        let interrupted = tracker
            .apply(hook("session-a", "Interrupt", "/work/alpha"))
            .expect("中断应回到空闲状态");
        assert_eq!(interrupted.event, "agent.idle");
        assert_eq!(interrupted.title.as_deref(), Some("INTERRUPTED"));
    }

    #[test]
    fn every_tracked_stop_announces_completion_with_parallel_tasks() {
        let mut tracker = CodexActivityTracker::default();
        for session in ["one", "two", "three"] {
            tracker.apply(hook(session, "UserPromptSubmit", "/work/project"));
        }

        let announcements = ["one", "two", "three"]
            .into_iter()
            .filter(|session| {
                let event = tracker
                    .apply(hook(session, "Stop", "/work/project"))
                    .expect("每个活动会话结束都应产生事件");
                event.event == "task.done"
                    || event
                        .extra
                        .get("announcement")
                        .and_then(|value| value.as_str())
                        == Some("done")
            })
            .count();

        assert_eq!(announcements, 3, "三个会话应分别触发三次完成播报");
    }
}
