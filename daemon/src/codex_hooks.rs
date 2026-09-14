use std::collections::HashMap;
use std::path::Path;

use beacon_protocol::{Event, VERSION};
use serde::Deserialize;
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;

#[derive(Debug, Deserialize)]
pub struct CodexHook {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivityStatus {
    Working,
    InputRequired,
}

#[derive(Clone, Debug)]
struct Activity {
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
                self.set_activity(&hook, ActivityStatus::Working);
                self.visible_activity()
            }
            "PermissionRequest" => {
                self.set_activity(&hook, ActivityStatus::InputRequired);
                self.visible_activity()
            }
            "PostToolUse" => {
                self.set_activity(&hook, ActivityStatus::Working);
                self.visible_activity()
            }
            "Stop" => {
                self.activities.remove(&hook.session_id);
                self.visible_activity().or_else(|| {
                    self.deduplicate(event("task.done", &hook.session_id, &project_title(&hook)))
                })
            }
            "Interrupt" => {
                self.activities.remove(&hook.session_id);
                self.visible_activity().or_else(|| {
                    self.deduplicate(event("agent.idle", &hook.session_id, "INTERRUPTED"))
                })
            }
            "SessionEnd" => {
                self.activities.remove(&hook.session_id);
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
            hook.session_id.clone(),
            Activity {
                status,
                title: project_title(hook),
                sequence: self.sequence,
            },
        );
    }

    fn visible_activity(&mut self) -> Option<Event> {
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
        self.deduplicate(visible)
    }

    fn deduplicate(&mut self, event: Event) -> Option<Event> {
        if self.last_visible.as_ref() == Some(&event) {
            return None;
        }
        self.last_visible = Some(event.clone());
        Some(event)
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
        CodexHook {
            session_id: session.to_owned(),
            hook_event_name: name.to_owned(),
            cwd: Some(cwd.to_owned()),
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
    fn interrupt_does_not_report_success() {
        let mut tracker = CodexActivityTracker::default();
        tracker.apply(hook("session-a", "UserPromptSubmit", "/work/alpha"));

        let interrupted = tracker
            .apply(hook("session-a", "Interrupt", "/work/alpha"))
            .expect("中断应回到空闲状态");
        assert_eq!(interrupted.event, "agent.idle");
        assert_eq!(interrupted.title.as_deref(), Some("INTERRUPTED"));
    }
}
