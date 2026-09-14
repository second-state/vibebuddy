//! 与具体 Agent 无关的活动聚合。
//!
//! Adapter 负责把某个 Agent 的事件翻译成这里的调用；任务卡排序、全局状态
//! 优先级、去重、一次性播报和过期清理都只在这里实现一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beacon_protocol::{Event, VERSION};
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;
/// 任务卡标题的显示上限，含区分 Agent 的前缀。
const MAX_TITLE_CHARS: usize = 26;
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
        self.idle_or_refresh(&id.session_id, idle_title)
    }

    /// 丢弃整个会话的活动，不播报成功。
    pub fn discard_session(&mut self, session_id: &str, idle_title: &str) -> Option<Event> {
        self.clear_session(session_id);
        self.idle_or_refresh(session_id, idle_title)
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
        self.idle_or_refresh(&expired_session, "TIMED OUT")
    }

    /// 活动清空时回到空闲，否则只刷新画面。
    ///
    /// 不能写成 `visible_activity().or_else(idle)`：`visible_activity` 返回
    /// `None` 有两种含义，没有活动和被去重吞掉，后者误报空闲会让仍在工作的
    /// 任务从屏幕上消失。
    fn idle_or_refresh(&mut self, session_id: &str, idle_title: &str) -> Option<Event> {
        if self.activities.is_empty() {
            self.deduplicate(event("agent.idle", session_id, idle_title))
        } else {
            self.visible_activity()
        }
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
///
/// `prefix` 区分是哪个 Agent 在跑，`fallback` 用于工作目录不可用时。
/// 两个 Agent 可能在同一个目录下工作，只有前缀能告诉用户该切到哪个窗口。
pub fn project_title(prefix: &str, cwd: Option<&str>, fallback: &str) -> String {
    let root = cwd.map(Path::new).and_then(project_root);
    let raw = root
        .as_deref()
        .or_else(|| cwd.map(Path::new))
        .and_then(|path| path.file_name())
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
        .take(MAX_TITLE_CHARS.saturating_sub(prefix.chars().count()))
        .collect();
    if title.is_empty() {
        format!("{prefix}{fallback}")
    } else {
        format!("{prefix}{title}")
    }
}

/// 从工作目录向上找到项目根。
///
/// 直接取工作目录的名字会把 `repo/tools` 显示成 TOOLS，把 git worktree 显示成
/// 分支目录名；用户认得项目名，不认得这两者。worktree 的 `.git` 是文件而非目录，
/// 内容指回主仓库，因此两种情况都能还原成同一个项目名。
fn project_root(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let git = dir.join(".git");
        if git.is_dir() {
            return Some(dir.to_path_buf());
        }
        if git.is_file() {
            let gitdir = std::fs::read_to_string(&git).ok()?;
            let gitdir = gitdir.strip_prefix("gitdir:")?.trim();
            return match gitdir.find("/.git/") {
                Some(index) => Some(PathBuf::from(&gitdir[..index])),
                None => Some(dir.to_path_buf()),
            };
        }
    }
    None
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
    fn ending_one_session_does_not_report_idle_while_another_works() {
        let mut tracker = ActivityTracker::default();
        let alive = id("alive", "alive:1");
        tracker.observe(&alive, "ALIVE", ActivityStatus::Working);

        // 另一个会话结束。它没有活动，画面也不该变化。
        let emitted = tracker.discard_session("other", "ALL QUIET");
        assert!(
            emitted.is_none(),
            "仍有活动时不得报告空闲，实际发出了 {emitted:?}"
        );
    }

    #[test]
    fn discarding_one_activity_does_not_report_idle_while_another_works() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("a", "a:1"), "ALPHA", ActivityStatus::Working);
        let other = id("b", "b:1");
        tracker.observe(&other, "BETA", ActivityStatus::Working);
        // 让 BETA 成为当前可见状态后再丢弃它，迫使剩余快照与上一次不同。
        let emitted = tracker.discard(&other, "INTERRUPTED");
        let emitted = emitted.expect("丢弃后应刷新为剩余活动");
        assert_eq!(emitted.event, "task.start", "仍有活动时不得报告空闲");
    }

    #[test]
    fn project_title_falls_back_when_cwd_is_unusable() {
        assert_eq!(
            project_title("CX:", Some("/work/agent-beacon"), "CODEX"),
            "CX:AGENT-BEACON"
        );
        assert_eq!(project_title("CX:", Some("/"), "CODEX"), "CX:CODEX");
        assert_eq!(project_title("CC:", None, "CLAUDE"), "CC:CLAUDE");
    }

    fn temp_tree(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("agentbeacon-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn title_uses_the_project_root_not_the_working_directory() {
        let base = temp_tree("root");
        let repo = base.join("my-project");
        let nested = repo.join("tools");
        std::fs::create_dir_all(&nested).expect("创建测试目录");
        std::fs::create_dir_all(repo.join(".git")).expect("创建 .git 目录");

        let title = project_title("CC:", nested.to_str(), "CLAUDE");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(title, "CC:MY-PROJECT", "子目录不应成为任务卡标题");
    }

    #[test]
    fn title_resolves_a_worktree_back_to_the_main_repository() {
        let base = temp_tree("worktree");
        let repo = base.join("my-project");
        let worktree = repo.join(".claude").join("worktrees").join("branch-xyz");
        std::fs::create_dir_all(&worktree).expect("创建 worktree 目录");
        std::fs::create_dir_all(repo.join(".git")).expect("创建 .git 目录");
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}/.git/worktrees/branch-xyz\n", repo.display()),
        )
        .expect("写入 worktree 的 .git");

        let title = project_title("CC:", worktree.to_str(), "CLAUDE");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(title, "CC:MY-PROJECT", "worktree 应显示主仓库名");
    }

    #[test]
    fn title_stays_within_the_display_limit() {
        let long = project_title("CC:", Some("/work/a-very-long-project-name-here"), "CLAUDE");
        assert!(
            long.chars().count() <= MAX_TITLE_CHARS,
            "标题不得超过显示上限"
        );
        assert!(long.starts_with("CC:"));
    }
}
