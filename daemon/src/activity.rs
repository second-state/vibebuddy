//! 与具体 Agent 无关的活动聚合。
//!
//! Adapter 负责把某个 Agent 的事件翻译成这里的调用；任务卡排序、全局状态
//! 优先级、去重、一次性播报和过期清理都只在这里实现一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vibebuddy_protocol::{Event, VERSION};
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
    Codex {
        thread_id: String,
    },
    /// `session_id` 是 CLI 的会话 id，它不足以定位窗口：worktree 迁移、fork 或
    /// 一次 resume 导入都会让同一个 id 对应多个桌面会话。`cwd` 是消歧的钥匙。
    ClaudeCode {
        session_id: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    GitHubActions {
        repo: String,
        run_id: u64,
    },
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

/// 当日战绩的快照：完成数、需要确认次数、忙碌秒数。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TodaySummary {
    pub done: u32,
    pub asks: u32,
    pub busy_seconds: u64,
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
    /// 最近一次播报过结束的活动。它一直是 K2 的落点，直到下一次播报把它换
    /// 掉——用墙上时钟让它过期是错的：这台设备的用处恰恰在于人不在电脑前，
    /// 去接杯水回来再按，落点不该已经飘走。
    recently_announced: Option<ActivitySource>,
    /// Agent 最近工作过的项目根及最后一次看到的时间。
    workspaces: HashMap<PathBuf, Instant>,
    /// 每个活动所属的项目名。标题第一行让给了会话名，项目名挪到第二行。
    projects: HashMap<String, String>,
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

    /// 记下刚播报过结束的活动，让 K2 在播报之后的短时间内仍能回到它。
    fn remember_announced(&mut self, activity: Activity) {
        if let Some(source) = activity.source {
            self.recently_announced = Some(source);
        }
    }

    /// K2 的落点。与屏幕主状态同源，但多一条：最近播报过结束的活动排在当前
    /// 工作项之前，并一直保持到下一次播报——用户是听到播报才去按的键，而那
    /// 件事此刻已经不在屏幕上了。
    pub fn focus_source(&self) -> Option<ActivitySource> {
        self.focus_sources().into_iter().next()
    }

    /// K2 的候选落点，按优先级排列、去重。第一个打不开（例如线程已不存在）
    /// 就试下一个，而不是打开一个空白窗口。
    pub fn focus_sources(&self) -> Vec<ActivitySource> {
        let mut sources: Vec<ActivitySource> = Vec::new();
        let mut push = |source: Option<ActivitySource>| {
            if let Some(source) = source
                && !sources.contains(&source)
            {
                sources.push(source);
            }
        };
        // 有任务在等人回答，那件事最急，先去那里。
        push(self.waiting_activity().and_then(|waiting| waiting.source.clone()));
        // 播报过结束的那件事已经离开卡片栈，屏幕上再也看不到它；而还在跑的
        // 任务一直挂在屏幕上，本来就不需要 K2 帮忙定位。
        push(self.recently_announced.clone());
        push(self.focused_activity().and_then(|activity| activity.source.clone()));
        push(self.last_source.clone());
        sources
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
        let Some(finished) = self.activities.remove(&id.key) else {
            return self.visible_activity();
        };
        self.remember_announced(finished);
        self.sync_busy();
        self.record(|stats| stats.done += 1);
        self.announce_end("task.done", id, title, "done")
    }

    /// 活动以失败告终，产生一次失败播报。
    ///
    /// 与 `discard` 的区别是失败是任务的结果，必须让用户知道；`discard`
    /// 用于「不知道结果」的收尾，不播报。
    pub fn fail(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        let Some(failed) = self.activities.remove(&id.key) else {
            return self.visible_activity();
        };
        self.remember_announced(failed);
        self.sync_busy();
        self.announce_end("task.error", id, title, "failed")
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

    /// 记下活动所属的项目。要在 `observe` 之前调用：快照在 `observe` 里生成。
    pub fn note_project(&mut self, id: &ActivityId, project: &str) {
        self.projects.insert(id.key.clone(), project.to_owned());
        let activities = &self.activities;
        self.projects
            .retain(|key, _| key == &id.key || activities.contains_key(key));
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
        self.attach_tasks(&mut visible);
        Some(visible)
    }

    /// 把当前卡片栈挂到事件上。结束播报也要带：用户在听到"完成"的同时，
    /// 应该看得见还剩什么在跑。
    fn attach_tasks(&self, visible: &mut Event) {
        let mut activities: Vec<(&String, &Activity)> = self.activities.iter().collect();
        activities.sort_by_key(|(_, activity)| std::cmp::Reverse(activity.sequence));
        visible.extra.insert(
            "tasks".to_owned(),
            json!(
                activities
                    .into_iter()
                    .take(MAX_VISIBLE_TASKS)
                    .map(|(key, activity)| {
                        let mut task = json!({
                            "title": activity.title,
                            "status": match activity.status {
                                ActivityStatus::Working => "working",
                                ActivityStatus::InputRequired => "input_required",
                            },
                        });
                        if let Some(project) = self.projects.get(key) {
                            task["project"] = json!(project);
                        }
                        task
                    })
                    .collect::<Vec<_>>()
            ),
        );
    }

    /// 结束播报必须说清是谁结束了。
    ///
    /// 并行跑的时候，屏幕主状态属于另一件还在跑的事，把 "done" 挂到那个快照
    /// 上等于告诉用户那一件完成了——而它没有。之前单任务能用，只是因为栈空
    /// 时走的是另一条分支，标题恰好就是完成者。
    fn announce_end(
        &mut self,
        event_name: &str,
        id: &ActivityId,
        title: &str,
        announcement: &str,
    ) -> Option<Event> {
        // 还有任务在等人回答时，屏幕留给它：那件事要用户动手，而"完成"只是
        // 通知，播报一声就够了。与 K2 的落点规则同一套优先级。
        let mut announced = match self.waiting_activity() {
            Some(waiting) => event(
                "agent.input_required",
                &waiting.session_id,
                &waiting.title.clone(),
            ),
            None => event(event_name, &id.session_id, title),
        };
        self.attach_tasks(&mut announced);
        announced
            .extra
            .insert("announcement".to_owned(), json!(announcement));
        announced
            .extra
            .insert("announcement_id".to_owned(), json!(id.key));
        // 屏幕现在停在这条播报上。下一个事件必须能把它刷回当前任务，所以这
        // 一帧不能当去重基准——否则状态没变的下一帧会被吞掉，屏幕卡在这里。
        self.last_visible = None;
        Some(announced)
    }

    /// 正在等人回答的活动里最新的那个。它 blocking 着用户，排在一切之前。
    fn waiting_activity(&self) -> Option<&Activity> {
        self.activities
            .values()
            .filter(|activity| activity.status == ActivityStatus::InputRequired)
            .max_by_key(|activity| activity.sequence)
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

    /// 当日战绩的数字形式，给 App 的状态接口用；空闲屏用的是 `stats_lines`。
    pub fn today(&self) -> TodaySummary {
        TodaySummary {
            done: self.stats.done,
            asks: self.stats.asks,
            busy_seconds: self.stats.busy_seconds
                + self
                    .stats
                    .busy_since
                    .map_or(0, |since| since.elapsed().as_secs()),
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
    display_title(prefix, project_name(cwd).as_deref().unwrap_or(fallback), fallback)
}

/// 工作目录所属项目的名字（项目根目录名）。
pub fn project_name(cwd: Option<&str>) -> Option<String> {
    let root = cwd.map(Path::new).and_then(project_root);
    root.as_deref()
        .or_else(|| cwd.map(Path::new))
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// 任务卡第一行：依次试 Agent 自己给会话起的名字，都不可用才写项目名。
///
/// “不可用”指滤掉设备字库画不出的字符之后剩不到三个字母数字——中文标题
/// 在这块只有大写字母和数字的屏上会变成空白，退回项目名比留白好。
pub fn card_title(
    prefix: &str,
    candidates: &[Option<String>],
    project: Option<&str>,
    fallback: &str,
) -> String {
    for candidate in candidates.iter().flatten() {
        let shown = display_title(prefix, candidate, "");
        let letters = shown[prefix.len()..]
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .count();
        if letters >= 3 {
            return shown;
        }
    }
    display_title(prefix, project.unwrap_or(fallback), fallback)
}

/// 把任意名字压成任务卡放得下的标题：只保留字母数字、连字符和单个空格，
/// 全大写。空格得留着：会话标题是几个词，粘在一起就读不出来了。
pub fn display_title(prefix: &str, raw: &str, fallback: &str) -> String {
    let mut title = String::new();
    for character in raw.chars() {
        if character.is_ascii_alphanumeric() {
            title.push(character.to_ascii_uppercase());
        } else if character == '-' || character == '_' {
            title.push(character);
        } else if character.is_whitespace() && !title.is_empty() && !title.ends_with(' ') {
            title.push(' ');
        }
    }
    let title: String = title
        .trim_end()
        .chars()
        .take(MAX_TITLE_CHARS.saturating_sub(prefix.chars().count()))
        .collect();
    // 截断可能正好切在连字符或空格上，留着像少了半个词。
    let title = title.trim_end_matches([' ', '-', '_']);
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
                repo: "longzhi/vibe-buddy".to_owned(),
                run_id: 42,
            },
        );

        let waiting = id("waiting", "waiting:1");
        tracker.require_input(&waiting, "CX:VIBE-BUDDY");
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
            cwd: Some("/work/vibe-buddy".to_owned()),
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
            cwd: Some("/work/vibe-buddy".to_owned()),
        };
        tracker.associate_source(&turn, source.clone());

        tracker.finish(&turn, "CC:PROJECT");

        assert_eq!(
            tracker.focus_source(),
            Some(source),
            "任务刚完成后，K2 仍应能返回对应会话"
        );
    }

    /// 并行跑两个任务时，一个结束会播报，另一个还在跑。用户是听到播报才去
    /// 按 K2 的，落点必须是刚播报的那件事，而不是恰好还活着的另一件。
    #[test]
    fn k2_returns_to_the_task_that_just_announced() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let running = id("session-b", "session-b:turn");
        let finished_source = ActivitySource::Codex {
            thread_id: "thread-finished".to_owned(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(&finished, finished_source.clone());
        tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(
            &running,
            ActivitySource::Codex {
                thread_id: "thread-running".to_owned(),
            },
        );

        tracker.finish(&finished, "CX:ALPHA");

        assert_eq!(
            tracker.focus_source(),
            Some(finished_source),
            "K2 应回到刚播报完成的任务，而不是还在跑的那个"
        );
    }

    /// 落点由播报接力，不由时钟决定：第二个任务完成后，K2 改指它。
    #[test]
    fn the_next_announcement_takes_over_the_landing_spot() {
        let mut tracker = ActivityTracker::default();
        let first = id("session-a", "session-a:turn");
        let second = id("session-b", "session-b:turn");
        let second_source = ActivitySource::Codex {
            thread_id: "thread-second".to_owned(),
        };
        tracker.observe(&first, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(
            &first,
            ActivitySource::Codex {
                thread_id: "thread-first".to_owned(),
            },
        );
        tracker.observe(&second, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(&second, second_source.clone());

        tracker.finish(&first, "CX:ALPHA");
        tracker.finish(&second, "CX:BETA");

        assert_eq!(
            tracker.focus_source(),
            Some(second_source),
            "K2 应跟随最后一次播报"
        );
    }

    /// 人离开工位再回来按 K2，落点不该因为时间流逝而飘走。
    #[test]
    fn the_landing_spot_does_not_expire_on_its_own() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let running = id("session-b", "session-b:turn");
        let finished_source = ActivitySource::Codex {
            thread_id: "thread-finished".to_owned(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(&finished, finished_source.clone());
        tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(
            &running,
            ActivitySource::Codex {
                thread_id: "thread-running".to_owned(),
            },
        );
        tracker.finish(&finished, "CX:ALPHA");

        // 中间那个任务一直在跑，刷新多少次都不该把落点抢走。
        for _ in 0..5 {
            tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        }

        assert_eq!(
            tracker.focus_source(),
            Some(finished_source),
            "只有下一次播报能换掉落点"
        );
    }

    /// 等人回答的任务是 blocking 的，排在刚播报完成的任务之前。
    #[test]
    fn a_task_waiting_for_a_reply_outranks_the_announcement() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let waiting = id("session-b", "session-b:turn");
        let waiting_source = ActivitySource::Codex {
            thread_id: "thread-waiting".to_owned(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(
            &finished,
            ActivitySource::Codex {
                thread_id: "thread-finished".to_owned(),
            },
        );
        tracker.require_input(&waiting, "CX:BETA");
        tracker.associate_source(&waiting, waiting_source.clone());

        tracker.finish(&finished, "CX:ALPHA");

        assert_eq!(
            tracker.focus_source(),
            Some(waiting_source),
            "有人在等回答时，K2 先去那里"
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
    fn card_title_prefers_a_readable_session_name() {
        let candidates = [
            None,
            Some("Eros infra   morning triage".to_owned()),
            Some("k2-source-navigation".to_owned()),
        ];
        assert_eq!(
            card_title("CC:", &candidates, Some("vibe-buddy"), "CLAUDE"),
            "CC:EROS INFRA MORNING TRIA"
        );
        let branch = [Some("pomodoro-timer-feature-b509bd".to_owned())];
        assert_eq!(
            card_title("CC:", &branch, Some("vibe-buddy"), "CLAUDE"),
            "CC:POMODORO-TIMER-FEATURE"
        );
        // 中文标题在设备字库上是空白，退回下一个候选，再退回项目名。
        let chinese = [Some("制定两周交易计划".to_owned()), Some("PR 96".to_owned())];
        assert_eq!(
            card_title("CX:", &chinese, Some("eros-training-infra"), "CODEX"),
            "CX:PR 96"
        );
        let only_chinese = [Some("制定两周交易计划".to_owned())];
        assert_eq!(
            card_title("CX:", &only_chinese, Some("eros-training-infra"), "CODEX"),
            "CX:EROS-TRAINING-INFRA"
        );
        assert_eq!(card_title("CX:", &[], None, "CODEX"), "CX:CODEX");
    }

    #[test]
    fn tasks_carry_the_project_next_to_the_session_title() {
        let mut tracker = ActivityTracker::default();
        let id = ActivityId {
            session_id: "s".to_owned(),
            key: "s:1".to_owned(),
        };
        tracker.note_project(&id, "VIBE-BUDDY");
        let event = tracker
            .observe(&id, "CC:POMODORO TIMER", ActivityStatus::Working)
            .expect("首个活动应可见");
        assert_eq!(event.extra["tasks"][0]["title"], "CC:POMODORO TIMER");
        assert_eq!(event.extra["tasks"][0]["project"], "VIBE-BUDDY");
    }

    #[test]
    fn project_title_falls_back_when_cwd_is_unusable() {
        assert_eq!(
            project_title("CX:", Some("/work/vibe-buddy"), "CODEX"),
            "CX:VIBE-BUDDY"
        );
        assert_eq!(project_title("CX:", Some("/"), "CODEX"), "CX:CODEX");
        assert_eq!(project_title("CC:", None, "CLAUDE"), "CC:CLAUDE");
    }

    fn temp_tree(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("vibebuddy-{}-{tag}", std::process::id()));
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
