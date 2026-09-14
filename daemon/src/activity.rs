//! 与具体 Agent 无关的活动聚合。
//!
//! Adapter 负责把某个 Agent 的事件翻译成这里的调用；任务卡排序、全局状态
//! 优先级、去重、一次性播报和过期清理都只在这里实现一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use beacon_protocol::{Event, VERSION};
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;
/// 任务卡标题的显示上限，含区分 Agent 的前缀。
const MAX_TITLE_CHARS: usize = 26;
/// 工作中的活动若长时间没有任何事件，通常是 Agent 进程已经消失。
const WORKING_TTL: Duration = Duration::from_secs(30 * 60);
/// 等待用户回应可以持续很久，过期时间必须长到足够用户离开再回来。
const INPUT_REQUIRED_TTL: Duration = Duration::from_secs(4 * 60 * 60);
/// Agent 工作过的项目根保留多久。超过这段时间没人在那儿干活，就不必再
/// 关心它的 CI 了。
const WORKSPACE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    Working,
    InputRequired,
}

/// K2 可以切回的 Mac 来源。这里只保存打开窗口所需的最小定位信息，
/// 不保存 prompt、回复正文或命令内容。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivitySource {
    Codex { thread_id: String },
    ClaudeCode { session_id: String },
    GitHubActions { repo: String, run_id: u64 },
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
    /// 进入当前状态的时刻。与 `updated_at` 不同：工具事件每几秒刷新一次
    /// `updated_at`，但卡片要回答的是「这个 turn 跑了多久」「等了多久」。
    status_since: Instant,
    source: Option<ActivitySource>,
}

/// 空闲屏轮播的当日战绩。
#[derive(Debug, Default)]
struct DailyStats {
    day: String,
    done: u32,
    asks: u32,
    busy_seconds: u64,
    /// 当前这段「至少有一个活动」的起点；没有活动时为 `None`。
    busy_since: Option<Instant>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct StoredStats {
    day: String,
    done: u32,
    asks: u32,
    busy_seconds: u64,
    #[serde(default)]
    last_source: Option<ActivitySource>,
}

#[derive(Default)]
pub struct ActivityTracker {
    activities: HashMap<String, Activity>,
    sequence: u64,
    last_visible: Option<Event>,
    stats: DailyStats,
    stats_file: Option<PathBuf>,
    /// 最近一次可定位的 Agent/CI 来源。当前活动结束或 daemon 重启后，K2
    /// 仍应能回到刚才那件事，而不是变成一个只在“工作中”才有效的按钮。
    last_source: Option<ActivitySource>,
    /// Agent 最近工作过的项目根及最后一次看到的时间。
    workspaces: HashMap<PathBuf, Instant>,
}

impl ActivityTracker {
    /// 把战绩存到磁盘。只放在内存里的话，每次重启 daemon 数字都会归零，
    /// 而屏幕上写的是「今天」，归零后它显示的就是错的。
    pub fn with_stats_file(path: PathBuf) -> Self {
        let stored: StoredStats = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            stats: DailyStats {
                day: stored.day,
                done: stored.done,
                asks: stored.asks,
                busy_seconds: stored.busy_seconds,
                busy_since: None,
            },
            last_source: stored.last_source,
            stats_file: Some(path),
            ..Self::default()
        }
    }

    /// 记下 Agent 正在哪个项目里工作。
    ///
    /// 这是个与 Agent 无关的事实，解析工作也已经为任务卡标题做过一遍。
    /// CI 靠它自动得出该关注哪些仓库，用户因此不必维护一份仓库清单。
    pub fn note_workspace(&mut self, cwd: Option<&str>) {
        let Some(root) = cwd.map(Path::new).and_then(project_root) else {
            return;
        };
        self.workspaces.insert(root, Instant::now());
    }

    /// 最近有 Agent 活动的项目根。
    pub fn recent_workspaces(&mut self) -> Vec<PathBuf> {
        let now = Instant::now();
        self.workspaces
            .retain(|_, seen| now.saturating_duration_since(*seen) < WORKSPACE_TTL);
        self.workspaces.keys().cloned().collect()
    }

    /// 给活动补上 K2 所需的来源定位。Adapter 在翻译事件后调用，因此事件若已
    /// 把活动收起，这里会自然地成为 no-op。
    pub fn associate_source(&mut self, id: &ActivityId, source: ActivitySource) {
        if let Some(activity) = self.activities.get_mut(&id.key) {
            activity.source = Some(source.clone());
            if self.last_source.as_ref() != Some(&source) {
                self.last_source = Some(source);
                self.save_stats();
            }
        }
    }

    /// 与屏幕主状态使用同一套选择规则：需要输入优先，其次才是最新的工作项。
    pub fn focus_source(&self) -> Option<ActivitySource> {
        match self.focused_activity() {
            Some(activity) => activity.source.clone(),
            None => self.last_source.clone(),
        }
    }

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
        self.record(|stats| stats.asks += 1);
        let visible = self.activity_snapshot()?;
        self.last_visible = Some(visible.clone());
        Some(visible)
    }

    /// 活动正常结束，产生一次完成播报；未被跟踪的活动只刷新画面。
    pub fn finish(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        if self.activities.remove(&id.key).is_none() {
            return self.visible_activity();
        }
        self.sync_busy();
        self.record(|stats| stats.done += 1);
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

    /// 活动以失败告终，产生一次失败播报。
    ///
    /// 与 `discard` 的区别是失败是任务的结果，必须让用户知道；`discard`
    /// 用于「不知道结果」的收尾，不播报。
    pub fn fail(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        if self.activities.remove(&id.key).is_none() {
            return self.visible_activity();
        }
        self.sync_busy();
        if let Some(visible) = self.activity_snapshot() {
            self.last_visible = Some(visible.clone());
            let mut announced = visible;
            announced
                .extra
                .insert("announcement".to_owned(), json!("failed"));
            announced
                .extra
                .insert("announcement_id".to_owned(), json!(id.key));
            Some(announced)
        } else {
            self.deduplicate(event("task.error", &id.key, title))
        }
    }

    /// 丢弃一个活动，不播报成功。
    pub fn discard(&mut self, id: &ActivityId, idle_title: &str) -> Option<Event> {
        self.activities.remove(&id.key);
        self.sync_busy();
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
        self.sync_busy();
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
        self.sync_busy();
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
        let now = Instant::now();
        // 状态没变就保留起点，否则每次 PostToolUse 都会把计时清零。
        let status_since = self
            .activities
            .get(&id.key)
            .filter(|existing| existing.status == status)
            .map_or(now, |existing| existing.status_since);
        let source = self
            .activities
            .get(&id.key)
            .and_then(|existing| existing.source.clone());
        self.activities.insert(
            id.key.clone(),
            Activity {
                session_id: id.session_id.clone(),
                status,
                title: title.to_owned(),
                sequence: self.sequence,
                updated_at: now,
                status_since,
                source,
            },
        );
        self.sync_busy();
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
        let activity = self.focused_activity()?;

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

    fn focused_activity(&self) -> Option<&Activity> {
        self.activities.values().max_by_key(|activity| {
            let priority = match activity.status {
                ActivityStatus::InputRequired => 2,
                ActivityStatus::Working => 1,
            };
            (priority, activity.sequence)
        })
    }

    /// 盖上只在发送这一刻才有意义的字段：卡片计时和当日战绩。
    ///
    /// 它们不能进 `activity_snapshot`，因为那份快照要参与去重。这两个字段
    /// 每秒都在变，一旦进入快照，每个工具事件都会绕过去重变成一帧重绘，
    /// 把工作中的动画不断打回第一帧。
    pub fn stamp_live_fields(&mut self, event: &mut Event) {
        self.roll_day();
        event
            .extra
            .insert("stats".to_owned(), json!(self.stats_lines()));

        let now = Instant::now();
        let mut activities: Vec<&Activity> = self.activities.values().collect();
        activities.sort_by_key(|activity| std::cmp::Reverse(activity.sequence));
        let Some(tasks) = event
            .extra
            .get_mut("tasks")
            .and_then(|tasks| tasks.as_array_mut())
        else {
            return;
        };
        for (task, activity) in tasks.iter_mut().zip(activities) {
            let Some(task) = task.as_object_mut() else {
                continue;
            };
            task.insert(
                "elapsed_s".to_owned(),
                json!(
                    now.saturating_duration_since(activity.status_since)
                        .as_secs()
                ),
            );
        }
    }

    fn stats_lines(&self) -> Vec<String> {
        let busy = self.stats.busy_seconds
            + self
                .stats
                .busy_since
                .map_or(0, |since| since.elapsed().as_secs());
        vec![
            format!("{} DONE", self.stats.done),
            format!("{} ASKS", self.stats.asks),
            format!("{} BUSY", format_duration(busy)),
        ]
    }

    fn record(&mut self, change: impl FnOnce(&mut DailyStats)) {
        self.roll_day();
        change(&mut self.stats);
        self.save_stats();
    }

    /// 跨过本地自然日就清零。屏幕上写的是「今天」，就必须按今天算。
    fn roll_day(&mut self) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if self.stats.day == today {
            return;
        }
        self.stats.day = today;
        self.stats.done = 0;
        self.stats.asks = 0;
        self.stats.busy_seconds = 0;
        if self.stats.busy_since.is_some() {
            // 跨零点时仍在进行的活动从零点重新计时，不把昨天算进今天。
            self.stats.busy_since = Some(Instant::now());
        }
    }

    /// 维护「至少有一个活动」的累计时长。
    fn sync_busy(&mut self) {
        match (self.activities.is_empty(), self.stats.busy_since) {
            (false, None) => self.stats.busy_since = Some(Instant::now()),
            (true, Some(since)) => {
                self.stats.busy_seconds += since.elapsed().as_secs();
                self.stats.busy_since = None;
                self.save_stats();
            }
            _ => {}
        }
    }

    fn save_stats(&self) {
        let Some(path) = self.stats_file.as_ref() else {
            return;
        };
        let stored = StoredStats {
            day: self.stats.day.clone(),
            done: self.stats.done,
            asks: self.stats.asks,
            busy_seconds: self.stats.busy_seconds,
            last_source: self.last_source.clone(),
        };
        let Ok(text) = serde_json::to_string(&stored) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 战绩丢了只是少一行展示，不值得让事件下发失败。
        let _ = std::fs::write(path, text);
    }

    fn deduplicate(&mut self, event: Event) -> Option<Event> {
        if self.last_visible.as_ref() == Some(&event) {
            return None;
        }
        self.last_visible = Some(event.clone());
        Some(event)
    }
}

/// 战绩行里的时长：一小时以内只给分钟，超过就给 `1H23`。
fn format_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    if minutes < 60 {
        format!("{minutes}M")
    } else {
        format!("{}H{:02}", minutes / 60, minutes % 60)
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
    display_title(prefix, raw, fallback)
}

/// 把任意名字压成任务卡放得下的标题：只保留字母数字和连字符，全大写。
pub fn display_title(prefix: &str, raw: &str, fallback: &str) -> String {
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
    fn k2_targets_the_same_high_priority_activity_as_the_screen() {
        let mut tracker = ActivityTracker::default();
        let working = id("working", "working:1");
        tracker.observe(&working, "CI:BUILD", ActivityStatus::Working);
        tracker.associate_source(
            &working,
            ActivitySource::GitHubActions {
                repo: "longzhi/agent-beacon".to_owned(),
                run_id: 42,
            },
        );

        let waiting = id("waiting", "waiting:1");
        tracker.require_input(&waiting, "CX:AGENT-BEACON");
        let codex = ActivitySource::Codex {
            thread_id: "waiting".to_owned(),
        };
        tracker.associate_source(&waiting, codex.clone());

        assert_eq!(tracker.focus_source(), Some(codex));
    }

    #[test]
    fn source_survives_activity_refreshes() {
        let mut tracker = ActivityTracker::default();
        let turn = id("session", "session:turn");
        tracker.observe(&turn, "CC:PROJECT", ActivityStatus::Working);
        let source = ActivitySource::ClaudeCode {
            session_id: "session-a".to_owned(),
        };
        tracker.associate_source(&turn, source.clone());

        tracker.observe(&turn, "CC:PROJECT", ActivityStatus::Working);

        assert_eq!(tracker.focus_source(), Some(source));
    }

    #[test]
    fn k2_falls_back_to_the_last_completed_source() {
        let mut tracker = ActivityTracker::default();
        let turn = id("session", "session:turn");
        tracker.observe(&turn, "CC:PROJECT", ActivityStatus::Working);
        let source = ActivitySource::ClaudeCode {
            session_id: "session-a".to_owned(),
        };
        tracker.associate_source(&turn, source.clone());

        tracker.finish(&turn, "CC:PROJECT");

        assert_eq!(
            tracker.focus_source(),
            Some(source),
            "任务刚完成后，K2 仍应能返回对应会话"
        );
    }

    #[test]
    fn k2_source_survives_daemon_restart() {
        let base = temp_tree("last-source");
        let state_file = base.join("stats.json");
        let source = ActivitySource::Codex {
            thread_id: "thread-a".to_owned(),
        };
        {
            let mut tracker = ActivityTracker::with_stats_file(state_file.clone());
            let turn = id("session", "session:turn");
            tracker.observe(&turn, "CX:PROJECT", ActivityStatus::Working);
            tracker.associate_source(&turn, source.clone());
        }

        let restored = ActivityTracker::with_stats_file(state_file);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(
            restored.focus_source(),
            Some(source),
            "daemon 重启不应让 K2 忘记最近会话"
        );
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
    fn the_card_counts_from_the_status_change_not_the_last_event() {
        let mut tracker = ActivityTracker::default();
        let turn = id("s", "s:1");
        tracker.observe(&turn, "ALPHA", ActivityStatus::Working);
        tracker
            .activities
            .get_mut("s:1")
            .expect("活动应存在")
            .status_since = Instant::now() - Duration::from_secs(600);

        let mut snapshot = tracker.activity_snapshot().expect("应有可见活动");
        tracker.stamp_live_fields(&mut snapshot);

        assert_eq!(
            snapshot.extra["tasks"][0]["elapsed_s"].as_u64(),
            Some(600),
            "卡片要回答这个 turn 跑了多久，而不是上一个工具事件多久以前"
        );
    }

    #[test]
    fn a_repeated_tool_event_stays_deduplicated_while_the_card_counts() {
        let mut tracker = ActivityTracker::default();
        let turn = id("s", "s:1");
        tracker.observe(&turn, "ALPHA", ActivityStatus::Working);
        tracker
            .activities
            .get_mut("s:1")
            .expect("活动应存在")
            .status_since = Instant::now() - Duration::from_secs(600);

        assert!(
            tracker
                .observe(&turn, "ALPHA", ActivityStatus::Working)
                .is_none(),
            "计时不得绕过去重：每个工具事件都重绘会把工作中的动画打回第一帧"
        );
    }

    #[test]
    fn a_failed_activity_reports_an_error_instead_of_success() {
        let mut tracker = ActivityTracker::default();
        let broken = id("ci", "ci:1");
        tracker.observe(&broken, "CI:ALPHA", ActivityStatus::Working);

        let failed = tracker.fail(&broken, "CI:ALPHA").expect("失败应可见");
        assert_eq!(failed.event, "task.error");
        assert_eq!(failed.title.as_deref(), Some("CI:ALPHA"));
    }

    #[test]
    fn a_failure_behind_other_work_still_announces() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("agent", "agent:1"), "CC:ALPHA", ActivityStatus::Working);
        let broken = id("ci", "ci:1");
        tracker.observe(&broken, "CI:ALPHA", ActivityStatus::Working);

        let failed = tracker.fail(&broken, "CI:ALPHA").expect("失败应可见");
        assert_eq!(
            failed
                .extra
                .get("announcement")
                .and_then(|value| value.as_str()),
            Some("failed"),
            "画面仍要显示别的任务，但失败不能被吞掉"
        );
    }

    #[test]
    fn stats_count_what_happened_today() {
        let mut tracker = ActivityTracker::default();
        let turn = id("s", "s:1");
        tracker.observe(&turn, "ALPHA", ActivityStatus::Working);
        tracker.require_input(&turn, "ALPHA");
        tracker.finish(&turn, "ALPHA");

        let lines = tracker.stats_lines();
        assert_eq!(lines[0], "1 DONE");
        assert_eq!(lines[1], "1 ASKS");
    }

    #[test]
    fn busy_time_stays_short_enough_for_one_line() {
        assert_eq!(format_duration(0), "0M");
        assert_eq!(format_duration(59 * 60), "59M");
        assert_eq!(format_duration(83 * 60), "1H23");
    }

    #[test]
    fn the_working_directory_reveals_the_project_without_being_configured() {
        let base = temp_tree("workspace");
        let repo = base.join("my-project");
        std::fs::create_dir_all(repo.join(".git")).expect("创建 .git 目录");
        std::fs::create_dir_all(repo.join("tools")).expect("创建子目录");

        let mut tracker = ActivityTracker::default();
        tracker.note_workspace(repo.join("tools").to_str());
        tracker.note_workspace(Some("/definitely/not/a/repository"));
        let seen = tracker.recent_workspaces();

        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(seen, vec![repo], "子目录应归到项目根，非仓库路径应忽略");
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
