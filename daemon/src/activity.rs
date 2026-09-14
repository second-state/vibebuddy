//! 与具体 Agent 无关的活动聚合。
//!
//! Adapter 负责把某个 Agent 的事件翻译成这里的调用；任务卡排序、全局状态
//! 优先级、去重、一次性播报和过期清理都只在这里实现一次。

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use beacon_protocol::{Event, VERSION};
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;
/// 工作中的活动若长时间没有任何事件，通常是 Agent 进程已经消失。
const WORKING_TTL: Duration = Duration::from_secs(30 * 60);
/// 等待用户回应可以持续很久，过期时间必须长到足够用户离开再回来。
const INPUT_REQUIRED_TTL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    Working,
    InputRequired,
}

/// 活动的身份。`key` 在所有会话中唯一，`session_id` 用于会话级操作。
#[derive(Clone, Debug)]
pub struct ActivityId {
    pub session_id: String,
    pub key: String,
}

#[derive(Clone, Debug)]
struct Activity {
    session_id: String,
    status: ActivityStatus,
    title: String,
    sequence: u64,
    updated_at: Instant,
}

#[derive(Default)]
pub struct ActivityTracker {
    activities: HashMap<String, Activity>,
    sequence: u64,
    last_visible: Option<Event>,
}

impl ActivityTracker {
    /// 记录活动的当前状态，返回需要下发的可见状态。
    pub fn observe(
        &mut self,
        id: &ActivityId,
        title: &str,
        status: ActivityStatus,
    ) -> Option<Event> {
        self.set_activity(id, title, status);
        self.visible_activity()
    }

    /// 标记活动正在等待用户回应。重复标记不会再次触发语音。
    pub fn require_input(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        if self
            .activities
            .get(&id.key)
            .is_some_and(|activity| activity.status == ActivityStatus::InputRequired)
        {
            return None;
        }
        self.set_activity(id, title, ActivityStatus::InputRequired);
        let visible = self.activity_snapshot()?;
        self.last_visible = Some(visible.clone());
        Some(visible)
    }

    /// 活动正常结束，产生一次完成播报；未被跟踪的活动只刷新画面。
    pub fn finish(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        if self.activities.remove(&id.key).is_none() {
            return self.visible_activity();
        }
        if let Some(visible) = self.activity_snapshot() {
            // 状态快照用于去重；announcement 是一次性边沿事件，不能被状态合并吞掉。
            self.last_visible = Some(visible.clone());
            let mut announced = visible;
            announced
                .extra
                .insert("announcement".to_owned(), json!("done"));
            announced
                .extra
                .insert("announcement_id".to_owned(), json!(id.key));
            Some(announced)
        } else {
            self.deduplicate(event("task.done", &id.key, title))
        }
    }

    /// 丢弃一个活动，不播报成功。
    pub fn discard(&mut self, id: &ActivityId, idle_title: &str) -> Option<Event> {
        self.activities.remove(&id.key);
        self.visible_activity()
            .or_else(|| self.deduplicate(event("agent.idle", &id.session_id, idle_title)))
    }

    /// 丢弃整个会话的活动，不播报成功。
    pub fn discard_session(&mut self, session_id: &str, idle_title: &str) -> Option<Event> {
        self.clear_session(session_id);
        self.visible_activity()
            .or_else(|| self.deduplicate(event("agent.idle", session_id, idle_title)))
    }

    /// 清除某个会话的既有活动，不产生事件。
    pub fn clear_session(&mut self, session_id: &str) {
        self.activities
            .retain(|_, activity| activity.session_id != session_id);
    }

    /// 清除已被遗弃的活动。Agent 被强制结束时不会发送收尾事件，
    /// 若没有过期机制，这些活动会永久占用任务卡并让宠物停在需要确认。
    pub fn sweep_expired(&mut self) -> Option<Event> {
        self.sweep_expired_at(Instant::now())
    }

    fn sweep_expired_at(&mut self, now: Instant) -> Option<Event> {
        let mut expired_session = None;
        self.activities.retain(|_, activity| {
            let ttl = match activity.status {
                ActivityStatus::Working => WORKING_TTL,
                ActivityStatus::InputRequired => INPUT_REQUIRED_TTL,
            };
            let alive = now.duration_since(activity.updated_at) < ttl;
            if !alive {
                expired_session = Some(activity.session_id.clone());
            }
            alive
        });
        let expired_session = expired_session?;
        self.visible_activity()
            .or_else(|| self.deduplicate(event("agent.idle", &expired_session, "TIMED OUT")))
    }

    fn set_activity(&mut self, id: &ActivityId, title: &str, status: ActivityStatus) {
        self.sequence = self.sequence.wrapping_add(1);
        self.activities.insert(
            id.key.clone(),
            Activity {
                session_id: id.session_id.clone(),
                status,
                title: title.to_owned(),
                sequence: self.sequence,
                updated_at: Instant::now(),
            },
        );
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
        let (_, activity) = self.activities.iter().max_by_key(|(_, activity)| {
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
        let mut visible = event(event_name, &activity.session_id, &activity.title);
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

/// 从工作目录派生任务卡标题。不读取 prompt 或会话内容。
/// `fallback` 由 Adapter 提供，用于工作目录不可用时仍能指出是哪个 Agent。
pub fn project_title(cwd: Option<&str>, fallback: &str) -> String {
    let raw = cwd
        .and_then(|cwd| Path::new(cwd).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or(fallback);
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
        fallback.to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(session: &str, key: &str) -> ActivityId {
        ActivityId {
            session_id: session.to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn abandoned_working_activity_expires_instead_of_holding_the_card() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("killed", "killed:1"), "ALPHA", ActivityStatus::Working);

        assert!(
            tracker
                .sweep_expired_at(Instant::now() + WORKING_TTL / 2)
                .is_none(),
            "未超时的工作任务不应被清除"
        );

        let expired = tracker
            .sweep_expired_at(Instant::now() + WORKING_TTL + Duration::from_secs(1))
            .expect("超时的工作任务应清空画面");
        assert_eq!(expired.event, "agent.idle");
        assert_eq!(expired.title.as_deref(), Some("TIMED OUT"));
    }

    #[test]
    fn waiting_activity_outlives_the_working_ttl() {
        let mut tracker = ActivityTracker::default();
        tracker.require_input(&id("waiting", "waiting:1"), "BETA");

        assert!(
            tracker
                .sweep_expired_at(Instant::now() + WORKING_TTL + Duration::from_secs(1))
                .is_none(),
            "用户可能离开很久，等待确认不能按工作中的时限清除"
        );

        let expired = tracker
            .sweep_expired_at(Instant::now() + INPUT_REQUIRED_TTL + Duration::from_secs(1))
            .expect("超过等待时限后应释放状态");
        assert_eq!(expired.event, "agent.idle");
    }

    #[test]
    fn expiring_one_task_keeps_the_others_visible() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("stale", "stale:1"), "STALE", ActivityStatus::Working);
        tracker.observe(&id("fresh", "fresh:1"), "FRESH", ActivityStatus::Working);
        tracker
            .activities
            .get_mut("fresh:1")
            .expect("活动应存在")
            .updated_at = Instant::now() + WORKING_TTL - Duration::from_secs(60);

        let refreshed = tracker
            .sweep_expired_at(Instant::now() + WORKING_TTL + Duration::from_secs(1))
            .expect("清除过期任务后应刷新卡片栈");
        assert_eq!(refreshed.event, "task.start");
        let tasks = refreshed.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "FRESH");
    }

    #[test]
    fn project_title_falls_back_when_cwd_is_unusable() {
        assert_eq!(
            project_title(Some("/work/agent-beacon"), "CODEX"),
            "AGENT-BEACON"
        );
        assert_eq!(project_title(Some("/"), "CODEX"), "CODEX");
        assert_eq!(project_title(None, "CODEX"), "CODEX");
    }
}
