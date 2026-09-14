use std::collections::HashMap;
use std::path::Path;

use beacon_protocol::{Event, VERSION};
use serde::Deserialize;
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivityStatus {
    Working,
    InputRequired,
}

#[derive(Clone, Debug)]
struct Activity {
    session_id: String,
    status: ActivityStatus,
    title: String,
    sequence: u64,
}

#[derive(Default)]
pub struct CodexActivityTracker {
    activities: HashMap<String, Activity>,
    sequence: u64,
    last_visible: Option<Event>,
}

impl CodexActivityTracker {
    pub fn apply(&mut self, hook: CodexHook) -> Option<Event> {
        match hook.hook_event_name.as_str() {
            "UserPromptSubmit" => {
                self.remove_session_activities(&hook.session_id);
                self.set_activity(&hook, ActivityStatus::Working);
                self.visible_activity()
            }
            "PermissionRequest" => self.require_input(&hook),
            "PostToolUse" => {
                self.set_activity(&hook, ActivityStatus::Working);
                self.visible_activity()
            }
            "Stop" => {
                if hook.response_kind.as_deref() == Some("input_required") {
                    return self.require_input(&hook);
                }
                let activity_id = activity_id(&hook);
                if self.activities.remove(&activity_id).is_none() {
                    return self.visible_activity();
                }
                self.announce_completion(&activity_id, &project_title(&hook))
            }
            "Interrupt" => {
                self.activities.remove(&activity_id(&hook));
                self.visible_activity().or_else(|| {
                    self.deduplicate(event("agent.idle", &hook.session_id, "INTERRUPTED"))
                })
            }
            "SessionEnd" => {
                self.activities
                    .retain(|_, activity| activity.session_id != hook.session_id);
                self.visible_activity().or_else(|| {
                    self.deduplicate(event("agent.idle", &hook.session_id, "ALL QUIET"))
                })
            }
            _ => None,
        }
    }

    fn set_activity(&mut self, hook: &CodexHook, status: ActivityStatus) {
        self.sequence = self.sequence.wrapping_add(1);
        self.activities.insert(
            activity_id(hook),
            Activity {
                session_id: hook.session_id.clone(),
                status,
                title: project_title(hook),
                sequence: self.sequence,
            },
        );
    }

    fn require_input(&mut self, hook: &CodexHook) -> Option<Event> {
        let activity_id = activity_id(hook);
        if self
            .activities
            .get(&activity_id)
            .is_some_and(|activity| activity.status == ActivityStatus::InputRequired)
        {
            return None;
        }
        self.set_activity(hook, ActivityStatus::InputRequired);
        let visible = self.activity_snapshot()?;
        self.last_visible = Some(visible.clone());
        Some(visible)
    }

    fn remove_session_activities(&mut self, session_id: &str) {
        self.activities
            .retain(|_, activity| activity.session_id != session_id);
    }

    fn visible_activity(&mut self) -> Option<Event> {
        let visible = self.activity_snapshot()?;
        let mut emitted = self.deduplicate(visible)?;
        if emitted.event == "agent.input_required" {
            emitted
                .extra
                .insert("suppress_audio".to_owned(), json!(true));
        }
        Some(emitted)
    }

    fn activity_snapshot(&self) -> Option<Event> {
        let (session_id, activity) = self.activities.iter().max_by_key(|(_, activity)| {
            let priority = match activity.status {
                ActivityStatus::InputRequired => 2,
                ActivityStatus::Working => 1,
            };
            (priority, activity.sequence)
        })?;

        let event_name = match activity.status {
            ActivityStatus::Working => "task.start",
            ActivityStatus::InputRequired => "agent.input_required",
        };
        let mut visible = event(event_name, session_id, &activity.title);
        let mut activities: Vec<&Activity> = self.activities.values().collect();
        activities.sort_by_key(|activity| std::cmp::Reverse(activity.sequence));
        visible.extra.insert(
            "tasks".to_owned(),
            json!(
                activities
                    .into_iter()
                    .take(MAX_VISIBLE_TASKS)
                    .map(|activity| {
                        json!({
                            "title": activity.title,
                            "status": match activity.status {
                                ActivityStatus::Working => "working",
                                ActivityStatus::InputRequired => "input_required",
                            },
                        })
                    })
                    .collect::<Vec<_>>()
            ),
        );
        Some(visible)
    }

    fn announce_completion(&mut self, activity_id: &str, title: &str) -> Option<Event> {
        if let Some(visible) = self.activity_snapshot() {
            // 状态快照用于去重；announcement 是一次性边沿事件，不能被状态合并吞掉。
            self.last_visible = Some(visible.clone());
            let mut announced = visible;
            announced
                .extra
                .insert("announcement".to_owned(), json!("done"));
            announced
                .extra
                .insert("announcement_id".to_owned(), json!(activity_id));
            Some(announced)
        } else {
            self.deduplicate(event("task.done", activity_id, title))
        }
    }

    fn deduplicate(&mut self, event: Event) -> Option<Event> {
        if self.last_visible.as_ref() == Some(&event) {
            return None;
        }
        self.last_visible = Some(event.clone());
        Some(event)
    }
}

fn activity_id(hook: &CodexHook) -> String {
    match hook.turn_id.as_deref() {
        Some(turn_id) => format!("{}:{turn_id}", hook.session_id),
        None => hook.session_id.clone(),
    }
}

fn event(name: &str, session_id: &str, title: &str) -> Event {
    Event {
        version: VERSION,
        event: name.to_owned(),
        id: Some(session_id.to_owned()),
        title: Some(title.to_owned()),
        message: None,
        extra: Default::default(),
    }
}

fn project_title(hook: &CodexHook) -> String {
    let raw = hook
        .cwd
        .as_deref()
        .and_then(|cwd| Path::new(cwd).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("CODEX");
    let title: String = raw
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_uppercase())
            } else if character == '-' || character == '_' {
                Some(character)
            } else {
                None
            }
        })
        .take(26)
        .collect();
    if title.is_empty() {
        "CODEX".to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
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
