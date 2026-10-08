//! Agent-agnostic activity aggregation.
//!
//! Adapters translate one agent's events into calls here; task-card ordering, global state
//! priority, dedup, one-shot announcements and expiry are implemented here exactly once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{NaiveDate, NaiveDateTime};
use vibebuddy_protocol::{Event, VERSION};

use crate::occasions;
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_VISIBLE_TASKS: usize = 3;
/// Display limit for task-card titles, including the prefix that tells agents apart.
const MAX_TITLE_CHARS: usize = 26;
/// A working activity with no events for a long time usually means the agent process is gone.
const WORKING_TTL: Duration = Duration::from_secs(30 * 60);
/// Waiting for the user can last a long time; the expiry must be long enough for them to leave and come back.
const INPUT_REQUIRED_TTL: Duration = Duration::from_secs(4 * 60 * 60);
/// A child activity holds back its parent's done announcement, so a lost end event costs more there: shorter.
const CHILD_TTL: Duration = Duration::from_secs(10 * 60);
/// How long to remember project roots an agent worked in. If nobody has worked there for this long,
/// its CI is no longer worth watching.
const WORKSPACE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    Working,
    InputRequired,
}

/// Where the agent process runs, which decides where K2 sends the user. The hook decides this — only it
/// can see the process environment; the daemon only routes and never recomputes it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "surface", rename_all = "snake_case")]
pub enum Surface {
    /// Running in the agent's own desktop app. Claude also carries a desktop session id to find the exact window;
    /// Codex only has a thread id, with no second identity.
    App {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desktop_session_id: Option<String>,
    },
    /// Running in some other app: a terminal, an editor's integrated terminal, or any host we haven't seen. This
    /// bundle id is the destination; the daemon doesn't need to know it. The agent's ttys, when it has
    /// any, let a terminal that can be asked about its tabs go to the session's own tab.
    Host {
        bundle_id: String,
        /// Nearest first: a terminal wrapper adds a pseudo-terminal of its own under the one the terminal names.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ttys: Vec<String>,
        /// Inside tmux the tty belongs to a pane; the pane is switched to through tmux instead.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tmux: Option<TmuxPane>,
    },
    /// No host app: SSH, daemons, sessions started by launchd. K2 has nowhere to go.
    Headless,
    /// Outside macOS: the agent's ancestor pids, nearest first. K2 focuses the window owned by the first of
    /// them that has one; with none (SSH, tmux) it is headless after all.
    Window { pids: Vec<u32> },
}

/// A tmux pane, by the server's socket and the pane id (`%3`), as the hook read them from `$TMUX` and `$TMUX_PANE`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TmuxPane {
    pub socket: String,
    pub pane: String,
}

impl TmuxPane {
    pub fn from_hook(socket: Option<String>, pane: Option<String>) -> Option<Self> {
        Some(Self { socket: socket?, pane: pane? })
    }
}

impl Default for Surface {
    /// Old state files were written by versions that only supported desktop apps; read them back as App to keep the old behavior.
    fn default() -> Self {
        Self::App { desktop_session_id: None }
    }
}

impl Surface {
    /// Combine the flat fields reported by the hook into a surface.
    pub fn from_hook(
        kind: Option<&str>,
        host_bundle_id: Option<String>,
        host_ttys: Option<Vec<String>>,
        tmux: Option<TmuxPane>,
        host_pids: Option<Vec<u32>>,
        desktop_session_id: Option<String>,
    ) -> Self {
        match kind {
            // Claims a host but gave no bundle id: nowhere to go, and falling back to App would jump to the wrong place.
            Some("host") => match host_bundle_id {
                Some(bundle_id) => Self::Host { bundle_id, ttys: host_ttys.unwrap_or_default(), tmux },
                None => Self::Headless,
            },
            Some("headless") => Self::Headless,
            Some("window") => match host_pids {
                Some(pids) if !pids.is_empty() => Self::Window { pids },
                _ => Self::Headless,
            },
            // Unrecognized values come from a hook newer than the daemon; treat them the old way.
            _ => Self::App { desktop_session_id },
        }
    }
}

/// A Mac source K2 can switch back to. Only the minimal location needed to open the window is stored,
/// never prompts, reply text or command contents.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivitySource {
    Codex {
        thread_id: String,
        #[serde(default)]
        surface: Surface,
    },
    /// `session_id` is the CLI session id, which isn't enough to locate a window: a worktree move, a fork or
    /// a resume import can all map one id to several desktop sessions. When running in the app, use the
    /// desktop session id carried by `Surface::App` for an exact match — Claude provides it itself;
    /// fall back to `cwd` for disambiguation only when an old hook doesn't report it.
    ClaudeCode {
        session_id: String,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        surface: Surface,
    },
    GitHubActions {
        repo: String,
        run_id: u64,
    },
    /// An agent with no desktop app to deeplink into (OpenCode, GitHub Copilot CLI): K2 can only go to where it runs.
    Cli {
        agent: String,
        session_id: String,
        #[serde(default)]
        surface: Surface,
    },
}

/// An activity's identity. `key` is unique across all sessions; `session_id` is for session-level operations.
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
    /// When the current state was entered. Unlike `updated_at`: tool events refresh `updated_at` every few
    /// seconds, but the card answers "how long has this turn been running" and "how long has it waited".
    status_since: Instant,
    source: Option<ActivitySource>,
    /// Work an agent delegated inside its own session (a Claude Code subagent). It gets a card but is not the
    /// user's task: its end is never announced, and while it runs the session's work isn't done.
    child: bool,
}

/// Snapshot of today's stats: done count, times input was needed, busy seconds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TodaySummary {
    pub done: u32,
    pub asks: u32,
    pub busy_seconds: u64,
}

/// Today's stats rotated on the idle screen.
#[derive(Debug, Default)]
struct DailyStats {
    day: String,
    done: u32,
    asks: u32,
    busy_seconds: u64,
    /// Start of the current stretch with at least one activity; `None` when there is none.
    busy_since: Option<Instant>,
    /// The night whose late-night line has been said; once a night, shared by done and needs input.
    late_night: Option<NaiveDate>,
    /// The day the daily greeting was said.
    greeted: Option<NaiveDate>,
    /// Today's busy hours a long-session line has been said for.
    long_session_hours: u64,
    /// When an Agent last did something, for telling a welcome back.
    last_activity: Option<NaiveDateTime>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct StoredStats {
    day: String,
    done: u32,
    asks: u32,
    busy_seconds: u64,
    #[serde(default)]
    last_source: Option<ActivitySource>,
    #[serde(default)]
    late_night: Option<NaiveDate>,
    #[serde(default)]
    greeted: Option<NaiveDate>,
    #[serde(default)]
    long_session_hours: u64,
    #[serde(default)]
    last_activity: Option<NaiveDateTime>,
}

#[derive(Default)]
pub struct ActivityTracker {
    activities: HashMap<String, Activity>,
    sequence: u64,
    last_visible: Option<Event>,
    stats: DailyStats,
    stats_file: Option<PathBuf>,
    /// The most recent locatable agent/CI source. After the current activity ends or the daemon restarts, K2
    /// should still take the user back to what just happened, not become a button that only works while "working".
    last_source: Option<ActivitySource>,
    /// The activity whose end was announced most recently. It stays K2's destination until the next announcement
    /// replaces it — expiring it by wall clock is wrong: this device is useful precisely when the user is away from
    /// the computer, and after fetching a glass of water the destination shouldn't have drifted away.
    recently_announced: Option<ActivitySource>,
    /// Project roots agents worked in recently, with the last time each was seen.
    workspaces: HashMap<PathBuf, Instant>,
    /// Project name for each activity. The title's first line goes to the session name; the project name moves to the second line.
    projects: HashMap<String, String>,
    /// The greeting or welcome back the last activity earned, not yet sent.
    pending_say: Option<Event>,
}

impl ActivityTracker {
    /// Persist the stats to disk. Kept only in memory, the numbers reset on every daemon restart,
    /// while the screen says "today" — after a reset it would be showing the wrong thing.
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
                late_night: stored.late_night,
                greeted: stored.greeted,
                long_session_hours: stored.long_session_hours,
                last_activity: stored.last_activity,
            },
            last_source: stored.last_source,
            stats_file: Some(path),
            ..Self::default()
        }
    }

    /// Record which project an agent is working in.
    ///
    /// This is an agent-agnostic fact, and the parsing was already done for the task-card title.
    /// CI uses it to work out which repos to watch automatically, so users don't maintain a repo list.
    pub fn note_workspace(&mut self, cwd: Option<&str>) {
        let Some(root) = cwd.map(Path::new).and_then(project_root) else {
            return;
        };
        self.workspaces.insert(root, Instant::now());
    }

    /// Project roots with recent agent activity.
    pub fn recent_workspaces(&mut self) -> Vec<PathBuf> {
        let now = Instant::now();
        self.workspaces
            .retain(|_, seen| now.saturating_duration_since(*seen) < WORKSPACE_TTL);
        self.workspaces.keys().cloned().collect()
    }

    /// Attach the source location K2 needs to an activity. Adapters call this after translating an event, so if the
    /// event already closed the activity, this naturally becomes a no-op.
    pub fn associate_source(&mut self, id: &ActivityId, source: ActivitySource) {
        if let Some(activity) = self.activities.get_mut(&id.key) {
            activity.source = Some(source.clone());
            if self.last_source.as_ref() != Some(&source) {
                self.last_source = Some(source);
                self.save_stats();
            }
        }
    }

    /// Remember the activity whose end was just announced, so K2 can still return to it shortly afterwards.
    fn remember_announced(&mut self, activity: Activity) {
        if let Some(source) = activity.source {
            self.recently_announced = Some(source);
        }
    }

    /// K2's destination. Same source as the screen's main state, plus one rule: the activity most recently announced
    /// as ended comes before the current work item and stays until the next announcement — the user presses the key
    /// because they heard the announcement, and that item is no longer on screen.
    pub fn focus_source(&self) -> Option<ActivitySource> {
        self.focus_sources().into_iter().next()
    }

    /// K2's candidate destinations, in priority order and deduplicated. If the first can't be opened (say the
    /// thread no longer exists), try the next instead of opening a blank window.
    pub fn focus_sources(&self) -> Vec<ActivitySource> {
        let mut sources: Vec<ActivitySource> = Vec::new();
        let mut push = |source: Option<ActivitySource>| {
            if let Some(source) = source
                && !sources.contains(&source)
            {
                sources.push(source);
            }
        };
        // A task waiting for an answer is the most urgent; go there first.
        push(self.waiting_activity().and_then(|waiting| waiting.source.clone()));
        // The item whose end was announced has left the card stack and is no longer visible; tasks still running
        // stay on screen, so they never needed K2's help to find.
        push(self.recently_announced.clone());
        push(self.focused_activity().and_then(|activity| activity.source.clone()));
        push(self.last_source.clone());
        sources
    }

    /// Record the activity's current state and return the visible state to send.
    pub fn observe(
        &mut self,
        id: &ActivityId,
        title: &str,
        status: ActivityStatus,
    ) -> Option<Event> {
        self.set_activity(id, title, status);
        self.visible_activity()
    }

    /// Like `observe`, for a child activity.
    pub fn observe_child(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        self.set_activity(id, title, ActivityStatus::Working);
        if let Some(activity) = self.activities.get_mut(&id.key) {
            activity.child = true;
        }
        self.visible_activity()
    }

    pub fn has_activity(&self, id: &ActivityId) -> bool {
        self.activities.contains_key(&id.key)
    }

    /// Mark the activity as waiting for the user. Marking it again doesn't trigger the voice again.
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
        let mut visible = self.activity_snapshot()?;
        self.last_visible = Some(visible.clone());
        let now = chrono::Local::now().naive_local();
        if let Some(occasion) = occasions::input_occasion(now, self.stats.late_night) {
            self.spend_late_night(now);
            visible.extra.insert("occasion".to_owned(), json!(occasion));
        }
        Some(visible)
    }

    /// The activity ended normally: announce completion once; untracked activities only refresh the screen.
    pub fn finish(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        let Some(finished) = self.activities.remove(&id.key) else {
            return self.visible_activity();
        };
        // The agent ended its turn to wait for work it delegated: the user's task is not done yet. The child's cards
        // keep the screen busy, and the turn that wraps up after the last child is the one announced.
        if !finished.child && self.has_children(&id.session_id) {
            self.sync_busy();
            return self.visible_activity();
        }
        self.remember_announced(finished);
        self.sync_busy();
        self.record(|stats| stats.done += 1);
        let mut announced = self.announce_end("task.done", id, title, "done")?;
        let now = chrono::Local::now().naive_local();
        let busy_hours = occasions::long_session_hours(self.today().busy_seconds);
        let long_session_due = busy_hours > self.stats.long_session_hours;
        if let Some(occasion) = occasions::done_occasion(now, self.stats.done, self.stats.late_night, long_session_due) {
            if occasion == "late_night_done" {
                self.spend_late_night(now);
            }
            if occasion == "long_session" {
                self.stats.long_session_hours = busy_hours;
                self.save_stats();
            }
            announced.extra.insert("occasion".to_owned(), json!(occasion));
        }
        Some(announced)
    }

    fn spend_late_night(&mut self, now: NaiveDateTime) {
        self.stats.late_night = occasions::night_of(now);
        self.save_stats();
    }

    /// An Agent did something: the first activity of the day is the daily greeting's other chance,
    /// and the first after hours of quiet is a welcome back. The line waits in `take_say`.
    fn note_activity(&mut self, now: NaiveDateTime) {
        let quiet = self.stats.last_activity.map(|last| now - last);
        self.stats.last_activity = Some(now);
        if self.pending_say.is_some() {
            return;
        }
        if let Some(greeting) = self.daily_greeting_at(now) {
            self.pending_say = Some(greeting);
        } else if quiet.is_some_and(|quiet| quiet >= occasions::WELCOME_BACK_QUIET) {
            let mut welcome = Event::named("buddy.say");
            welcome.extra.insert("occasion".to_owned(), json!("welcome_back"));
            self.pending_say = Some(welcome);
            self.save_stats();
        }
    }

    /// A line the last activity earned (the daily greeting or a welcome back), to send before the
    /// activity's own event.
    pub fn take_say(&mut self) -> Option<Event> {
        self.pending_say.take()
    }

    /// The daily greeting, the first time the link comes up or an Agent does something on a local
    /// calendar day; None once it has been said today.
    pub fn daily_greeting(&mut self) -> Option<Event> {
        self.daily_greeting_at(chrono::Local::now().naive_local())
    }

    fn daily_greeting_at(&mut self, now: NaiveDateTime) -> Option<Event> {
        if self.stats.greeted == Some(now.date()) {
            return None;
        }
        self.stats.greeted = Some(now.date());
        // A greeting counts as the buddy and the user meeting: no welcome back right after it.
        self.stats.last_activity = Some(now);
        self.save_stats();
        let mut greeting = Event::named("buddy.say");
        greeting.extra.insert("occasion".to_owned(), json!(occasions::greeting(now)));
        Some(greeting)
    }

    /// The activity ended in failure: announce the failure once.
    ///
    /// Unlike `discard`, failure is the task's outcome and the user must be told; `discard`
    /// is for wrap-ups where the outcome is unknown, and doesn't announce.
    pub fn fail(&mut self, id: &ActivityId, title: &str) -> Option<Event> {
        let Some(failed) = self.activities.remove(&id.key) else {
            return self.visible_activity();
        };
        self.remember_announced(failed);
        self.sync_busy();
        self.announce_end("task.error", id, title, "failed")
    }

    /// Drop an activity without announcing success.
    pub fn discard(&mut self, id: &ActivityId, idle_title: &str) -> Option<Event> {
        self.activities.remove(&id.key);
        self.sync_busy();
        self.idle_or_refresh(&id.session_id, idle_title)
    }

    /// Drop all of a session's activities without announcing success.
    pub fn discard_session(&mut self, session_id: &str, idle_title: &str) -> Option<Event> {
        self.clear_session(session_id);
        self.idle_or_refresh(session_id, idle_title)
    }

    /// Clear a session's existing activities without producing an event.
    pub fn clear_session(&mut self, session_id: &str) {
        self.activities
            .retain(|_, activity| activity.session_id != session_id);
        self.sync_busy();
    }

    /// A new turn replaces the session's previous one; children it started earlier keep running and keep their cards.
    pub fn clear_turns(&mut self, session_id: &str) {
        self.activities
            .retain(|_, activity| activity.session_id != session_id || activity.child);
        self.sync_busy();
    }

    fn has_children(&self, session_id: &str) -> bool {
        self.activities
            .values()
            .any(|activity| activity.child && activity.session_id == session_id)
    }

    /// Clear abandoned activities. An agent that is force-killed sends no wrap-up event;
    /// without expiry those activities would hold task cards forever and leave the pet stuck on needs-input.
    pub fn sweep_expired(&mut self) -> Option<Event> {
        self.sweep_expired_at(Instant::now())
    }

    fn sweep_expired_at(&mut self, now: Instant) -> Option<Event> {
        let mut expired_session = None;
        self.activities.retain(|_, activity| {
            let ttl = match activity.status {
                ActivityStatus::InputRequired => INPUT_REQUIRED_TTL,
                ActivityStatus::Working if activity.child => CHILD_TTL,
                ActivityStatus::Working => WORKING_TTL,
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

    /// Return to idle when no activities remain, otherwise just refresh the screen.
    ///
    /// Can't be written as `visible_activity().or_else(idle)`: `visible_activity` returning
    /// `None` means two things, no activity or swallowed by dedup, and reporting idle for the latter would make
    /// a still-working task vanish from the screen.
    fn idle_or_refresh(&mut self, session_id: &str, idle_title: &str) -> Option<Event> {
        if self.activities.is_empty() {
            self.deduplicate(event("agent.idle", session_id, idle_title))
        } else {
            self.visible_activity()
        }
    }

    /// Record the activity's project. Must be called before `observe`: the snapshot is built inside `observe`.
    pub fn note_project(&mut self, id: &ActivityId, project: &str) {
        self.projects.insert(id.key.clone(), project.to_owned());
        let activities = &self.activities;
        self.projects
            .retain(|key, _| key == &id.key || activities.contains_key(key));
    }

    fn set_activity(&mut self, id: &ActivityId, title: &str, status: ActivityStatus) {
        self.note_activity(chrono::Local::now().naive_local());
        self.sequence = self.sequence.wrapping_add(1);
        let now = Instant::now();
        // Keep the start time if the state hasn't changed, otherwise every PostToolUse would reset the timer.
        let status_since = self
            .activities
            .get(&id.key)
            .filter(|existing| existing.status == status)
            .map_or(now, |existing| existing.status_since);
        let existing = self.activities.get(&id.key);
        let source = existing.and_then(|existing| existing.source.clone());
        let child = existing.is_some_and(|existing| existing.child);
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
                child,
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

    /// Attach the current card stack to the event. End announcements carry it too: while hearing "done",
    /// the user should see what's still running.
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

    /// An end announcement must say who ended.
    ///
    /// When tasks run in parallel, the screen's main state belongs to another task still running; hanging "done" on that
    /// snapshot tells the user that one finished — and it didn't. It used to work with a single task only because an empty
    /// stack took a different branch, where the title happened to be the finisher.
    fn announce_end(
        &mut self,
        event_name: &str,
        id: &ActivityId,
        title: &str,
        announcement: &str,
    ) -> Option<Event> {
        // While a task is waiting for an answer, the screen stays on it: that item needs the user to act, while "done" is just
        // a notice, and one announcement is enough. Same priority rules as K2's destination.
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
        // The screen is now showing this announcement. The next event must be able to refresh it back to the current task, so
        // this frame can't be the dedup baseline — otherwise the next unchanged frame would be swallowed and the screen stuck here.
        self.last_visible = None;
        Some(announced)
    }

    /// The newest activity waiting for an answer. It is blocking the user and comes before everything else.
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

    /// Stamp the fields that only mean something at send time: card timers and today's stats.
    ///
    /// They can't go into `activity_snapshot`, because that snapshot takes part in dedup. Both fields
    /// change every second; in the snapshot, every tool event would bypass dedup and become a redraw,
    /// constantly knocking the working animation back to its first frame.
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

    /// Today's stats as numbers, for the app's status API; the idle screen uses `stats_lines`.
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

    /// Reset when the local calendar day changes. The screen says "today", so it has to count today.
    fn roll_day(&mut self) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if self.stats.day == today {
            return;
        }
        self.stats.day = today;
        self.stats.done = 0;
        self.stats.asks = 0;
        self.stats.busy_seconds = 0;
        self.stats.long_session_hours = 0;
        if self.stats.busy_since.is_some() {
            // Activities still running across midnight restart their timer at midnight; yesterday doesn't count toward today.
            self.stats.busy_since = Some(Instant::now());
        }
    }

    /// Maintain the accumulated time with at least one activity.
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
            late_night: self.stats.late_night,
            greeted: self.stats.greeted,
            long_session_hours: self.stats.long_session_hours,
            last_activity: self.stats.last_activity,
        };
        let Ok(text) = serde_json::to_string(&stored) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Losing the stats only costs one line of display; not worth failing the event send.
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

/// Duration in the stats line: minutes only under an hour, `1H23` above.
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

/// Derive the task-card title from the working directory. Never reads prompts or session content.
///
/// `prefix` tells which agent is running; `fallback` is for when the working directory is unavailable.
/// Two agents may work in the same directory, and only the prefix tells the user which window to switch to.
pub fn project_title(prefix: &str, cwd: Option<&str>, fallback: &str) -> String {
    display_title(prefix, project_name(cwd).as_deref().unwrap_or(fallback), fallback)
}

/// Name of the project the working directory belongs to (the project root's directory name).
pub fn project_name(cwd: Option<&str>) -> Option<String> {
    let root = cwd.map(Path::new).and_then(project_root);
    root.as_deref()
        .or_else(|| cwd.map(Path::new))
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// First line of a task card: try the agent's own session names in turn, and only fall back to the project name.
///
/// "Unavailable" means fewer than three alphanumerics left after filtering characters the device font can't draw — a Chinese
/// title would be blank on this uppercase-and-digits-only screen, and the project name beats blank space.
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

/// Squash any name into a title that fits a task card: keep only alphanumerics, hyphens and single spaces,
/// all uppercase. Spaces must stay: session titles are several words, and glued together they can't be read.
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
    // Truncation may land right on a hyphen or space; leaving it looks like half a word is missing.
    let title = title.trim_end_matches([' ', '-', '_']);
    if title.is_empty() {
        format!("{prefix}{fallback}")
    } else {
        format!("{prefix}{title}")
    }
}

/// Walk up from the working directory to the project root.
///
/// Taking the working directory's name directly would show `repo/tools` as TOOLS and a git worktree as the
/// branch directory name; users recognize the project name, not either of those. A worktree's `.git` is a file, not a directory,
/// pointing back at the main repo, so both cases resolve to the same project name.
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
            surface: Surface::default(),
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
            surface: Surface::default(),
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
            surface: Surface::default(),
        };
        tracker.associate_source(&turn, source.clone());

        tracker.finish(&turn, "CC:PROJECT");

        assert_eq!(
            tracker.focus_source(),
            Some(source),
            "right after a task finishes, K2 should still return to its session"
        );
    }

    /// With two tasks running in parallel, one finishes and is announced while the other keeps running. The user presses
    /// K2 because they heard the announcement, so the destination must be what was just announced, not whichever other one is still alive.
    #[test]
    fn k2_returns_to_the_task_that_just_announced() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let running = id("session-b", "session-b:turn");
        let finished_source = ActivitySource::Codex {
            thread_id: "thread-finished".to_owned(),
            surface: Surface::default(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(&finished, finished_source.clone());
        tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(
            &running,
            ActivitySource::Codex {
                thread_id: "thread-running".to_owned(),
                surface: Surface::default(),
            },
        );

        tracker.finish(&finished, "CX:ALPHA");

        assert_eq!(
            tracker.focus_source(),
            Some(finished_source),
            "K2 should return to the task just announced as done, not the one still running"
        );
    }

    /// The destination is handed on by announcements, not decided by the clock: after the second task finishes, K2 points at it.
    #[test]
    fn the_next_announcement_takes_over_the_landing_spot() {
        let mut tracker = ActivityTracker::default();
        let first = id("session-a", "session-a:turn");
        let second = id("session-b", "session-b:turn");
        let second_source = ActivitySource::Codex {
            thread_id: "thread-second".to_owned(),
            surface: Surface::default(),
        };
        tracker.observe(&first, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(
            &first,
            ActivitySource::Codex {
                thread_id: "thread-first".to_owned(),
                surface: Surface::default(),
            },
        );
        tracker.observe(&second, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(&second, second_source.clone());

        tracker.finish(&first, "CX:ALPHA");
        tracker.finish(&second, "CX:BETA");

        assert_eq!(
            tracker.focus_source(),
            Some(second_source),
            "K2 should follow the latest announcement"
        );
    }

    /// Leaving the desk and coming back to press K2, the destination shouldn't drift away just because time passed.
    #[test]
    fn the_landing_spot_does_not_expire_on_its_own() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let running = id("session-b", "session-b:turn");
        let finished_source = ActivitySource::Codex {
            thread_id: "thread-finished".to_owned(),
            surface: Surface::default(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(&finished, finished_source.clone());
        tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        tracker.associate_source(
            &running,
            ActivitySource::Codex {
                thread_id: "thread-running".to_owned(),
                surface: Surface::default(),
            },
        );
        tracker.finish(&finished, "CX:ALPHA");

        // The middle task keeps running; no number of refreshes should take the destination away.
        for _ in 0..5 {
            tracker.observe(&running, "CX:BETA", ActivityStatus::Working);
        }

        assert_eq!(
            tracker.focus_source(),
            Some(finished_source),
            "only the next announcement may change the target"
        );
    }

    /// A task waiting for an answer is blocking and comes before a task just announced as done.
    #[test]
    fn a_task_waiting_for_a_reply_outranks_the_announcement() {
        let mut tracker = ActivityTracker::default();
        let finished = id("session-a", "session-a:turn");
        let waiting = id("session-b", "session-b:turn");
        let waiting_source = ActivitySource::Codex {
            thread_id: "thread-waiting".to_owned(),
            surface: Surface::default(),
        };
        tracker.observe(&finished, "CX:ALPHA", ActivityStatus::Working);
        tracker.associate_source(
            &finished,
            ActivitySource::Codex {
                thread_id: "thread-finished".to_owned(),
                surface: Surface::default(),
            },
        );
        tracker.require_input(&waiting, "CX:BETA");
        tracker.associate_source(&waiting, waiting_source.clone());

        tracker.finish(&finished, "CX:ALPHA");

        assert_eq!(
            tracker.focus_source(),
            Some(waiting_source),
            "when someone is waiting for an answer, K2 goes there first"
        );
    }

    #[test]
    fn k2_source_survives_daemon_restart() {
        let base = temp_tree("last-source");
        let state_file = base.join("stats.json");
        let source = ActivitySource::Codex {
            thread_id: "thread-a".to_owned(),
            surface: Surface::default(),
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
            "a daemon restart should not make K2 forget the latest session"
        );
    }

    #[test]
    fn a_silent_child_expires_sooner_so_it_cannot_mute_its_parent_for_long() {
        let mut tracker = ActivityTracker::default();
        let parent = id("session", "session:turn");
        tracker.observe(&parent, "CC:MAIN", ActivityStatus::Working);
        tracker.observe_child(&id("session", "session:turn:agent"), "CC:MAIN");
        tracker.activities.get_mut(&parent.key).expect("parent should be tracked").updated_at += CHILD_TTL;

        tracker.sweep_expired_at(Instant::now() + CHILD_TTL + Duration::from_secs(1));
        let done = tracker.finish(&parent, "CC:MAIN").expect("finishing should be visible");
        assert_eq!(done.extra.get("announcement").and_then(|v| v.as_str()), Some("done"));
    }

    #[test]
    fn abandoned_working_activity_expires_instead_of_holding_the_card() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("killed", "killed:1"), "ALPHA", ActivityStatus::Working);

        assert!(
            tracker
                .sweep_expired_at(Instant::now() + WORKING_TTL / 2)
                .is_none(),
            "a working task that has not timed out should not be cleared"
        );

        let expired = tracker
            .sweep_expired_at(Instant::now() + WORKING_TTL + Duration::from_secs(1))
            .expect("a timed-out working task should clear the screen");
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
            "the user may be away for long; an input-required task must not be cleared on the working timeout"
        );

        let expired = tracker
            .sweep_expired_at(Instant::now() + INPUT_REQUIRED_TTL + Duration::from_secs(1))
            .expect("state should be released after the waiting timeout");
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
            .expect("activity should exist")
            .updated_at = Instant::now() + WORKING_TTL - Duration::from_secs(60);

        let refreshed = tracker
            .sweep_expired_at(Instant::now() + WORKING_TTL + Duration::from_secs(1))
            .expect("clearing expired tasks should refresh the card stack");
        assert_eq!(refreshed.event, "task.start");
        let tasks = refreshed.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "FRESH");
    }

    #[test]
    fn ending_one_session_does_not_report_idle_while_another_works() {
        let mut tracker = ActivityTracker::default();
        let alive = id("alive", "alive:1");
        tracker.observe(&alive, "ALIVE", ActivityStatus::Working);

        // Another session ends. It has no activity, and the screen shouldn't change.
        let emitted = tracker.discard_session("other", "ALL QUIET");
        assert!(
            emitted.is_none(),
            "must not report idle while activities remain, but emitted {emitted:?}"
        );
    }

    #[test]
    fn discarding_one_activity_does_not_report_idle_while_another_works() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("a", "a:1"), "ALPHA", ActivityStatus::Working);
        let other = id("b", "b:1");
        tracker.observe(&other, "BETA", ActivityStatus::Working);
        // Make BETA the current visible state, then drop it, forcing the remaining snapshot to differ from the last one.
        let emitted = tracker.discard(&other, "INTERRUPTED");
        let emitted = emitted.expect("after dropping, should refresh to the remaining activities");
        assert_eq!(emitted.event, "task.start", "must not report idle while activities remain");
    }

    #[test]
    fn the_card_counts_from_the_status_change_not_the_last_event() {
        let mut tracker = ActivityTracker::default();
        let turn = id("s", "s:1");
        tracker.observe(&turn, "ALPHA", ActivityStatus::Working);
        tracker
            .activities
            .get_mut("s:1")
            .expect("activity should exist")
            .status_since = Instant::now() - Duration::from_secs(600);

        let mut snapshot = tracker.activity_snapshot().expect("there should be a visible activity");
        tracker.stamp_live_fields(&mut snapshot);

        assert_eq!(
            snapshot.extra["tasks"][0]["elapsed_s"].as_u64(),
            Some(600),
            "the card should show how long this turn has run, not how long ago the last tool event was"
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
            .expect("activity should exist")
            .status_since = Instant::now() - Duration::from_secs(600);

        assert!(
            tracker
                .observe(&turn, "ALPHA", ActivityStatus::Working)
                .is_none(),
            "timing must not bypass dedup: redrawing on every tool event would reset the working animation to its first frame"
        );
    }

    #[test]
    fn a_failed_activity_reports_an_error_instead_of_success() {
        let mut tracker = ActivityTracker::default();
        let broken = id("ci", "ci:1");
        tracker.observe(&broken, "CI:ALPHA", ActivityStatus::Working);

        let failed = tracker.fail(&broken, "CI:ALPHA").expect("failure should be visible");
        assert_eq!(failed.event, "task.error");
        assert_eq!(failed.title.as_deref(), Some("CI:ALPHA"));
    }

    #[test]
    fn a_failure_behind_other_work_still_announces() {
        let mut tracker = ActivityTracker::default();
        tracker.observe(&id("agent", "agent:1"), "CC:ALPHA", ActivityStatus::Working);
        let broken = id("ci", "ci:1");
        tracker.observe(&broken, "CI:ALPHA", ActivityStatus::Working);

        let failed = tracker.fail(&broken, "CI:ALPHA").expect("failure should be visible");
        assert_eq!(
            failed
                .extra
                .get("announcement")
                .and_then(|value| value.as_str()),
            Some("failed"),
            "the screen should still show other tasks, but the failure must not be swallowed"
        );
    }

    #[test]
    fn the_first_done_of_the_day_is_a_special_occasion() {
        let mut tracker = ActivityTracker::default();
        let mut occasions = Vec::new();
        for turn in 0..5 {
            let turn = id("s", &format!("s:{turn}"));
            tracker.observe(&turn, "ALPHA", ActivityStatus::Working);
            let done = tracker.finish(&turn, "ALPHA").expect("a done is announced");
            occasions.push(done.extra.get("occasion").and_then(|value| value.as_str()).map(str::to_owned));
        }
        // Late at night the first done is the late-night one instead; either way the 5th is a milestone.
        let first = occasions[0].as_deref();
        assert!(matches!(first, Some("first_done" | "late_night_done")), "{first:?}");
        assert_eq!(occasions[1..4], [None, None, None]);
        assert_eq!(occasions[4].as_deref(), Some("milestone"));
    }

    #[test]
    fn the_greeting_is_said_once_a_day() {
        let mut tracker = ActivityTracker::default();
        let morning = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap().and_hms_opt(9, 0, 0).unwrap();
        let greeting = tracker.daily_greeting_at(morning).expect("the first link of the day");
        assert_eq!(greeting.event, "buddy.say");
        assert_eq!(greeting.extra.get("occasion"), Some(&json!("greeting_morning")));
        assert!(tracker.daily_greeting_at(morning + Duration::from_secs(3600)).is_none(), "the box restarted");
        let next_evening = NaiveDate::from_ymd_opt(2026, 10, 7).unwrap().and_hms_opt(20, 0, 0).unwrap();
        let greeting = tracker.daily_greeting_at(next_evening).expect("a new day");
        assert_eq!(greeting.extra.get("occasion"), Some(&json!("greeting_evening")));
    }

    #[test]
    fn the_first_activity_of_the_day_greets_and_hours_of_quiet_welcome_back() {
        let mut tracker = ActivityTracker::default();
        let day = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let say = |tracker: &mut ActivityTracker, hour, minute| {
            tracker.note_activity(day.and_hms_opt(hour, minute, 0).unwrap());
            tracker.take_say().and_then(|say| say.extra.get("occasion").and_then(|o| o.as_str()).map(str::to_owned))
        };
        assert_eq!(say(&mut tracker, 9, 0).as_deref(), Some("greeting_morning"), "the box was plugged in all night");
        assert_eq!(say(&mut tracker, 9, 5), None);
        assert_eq!(say(&mut tracker, 11, 59), None, "under three hours");
        assert_eq!(say(&mut tracker, 15, 30).as_deref(), Some("welcome_back"));
        assert_eq!(say(&mut tracker, 15, 31), None);

        // Greeted when the link came up in the morning: the first activity soon after is no welcome back,
        // however long the night was.
        let mut tracker = ActivityTracker::default();
        tracker.stats.last_activity = Some(day.and_hms_opt(1, 0, 0).unwrap());
        assert!(tracker.daily_greeting_at(day.and_hms_opt(8, 50, 0).unwrap()).is_some());
        assert_eq!(say(&mut tracker, 9, 0), None);
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
        std::fs::create_dir_all(repo.join(".git")).expect("create .git directory");
        std::fs::create_dir_all(repo.join("tools")).expect("create subdirectory");

        let mut tracker = ActivityTracker::default();
        tracker.note_workspace(repo.join("tools").to_str());
        tracker.note_workspace(Some("/definitely/not/a/repository"));
        let seen = tracker.recent_workspaces();

        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(seen, vec![repo], "subdirectories should map to the project root, non-repo paths should be ignored");
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
        // Chinese titles are blank in the device font; fall back to the next candidate, then to the project name.
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
            .expect("first activity should be visible");
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
        std::fs::create_dir_all(&nested).expect("create test directory");
        std::fs::create_dir_all(repo.join(".git")).expect("create .git directory");

        let title = project_title("CC:", nested.to_str(), "CLAUDE");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(title, "CC:MY-PROJECT", "a subdirectory should not become the task card title");
    }

    #[test]
    fn title_resolves_a_worktree_back_to_the_main_repository() {
        let base = temp_tree("worktree");
        let repo = base.join("my-project");
        let worktree = repo.join(".claude").join("worktrees").join("branch-xyz");
        std::fs::create_dir_all(&worktree).expect("create worktree directory");
        std::fs::create_dir_all(repo.join(".git")).expect("create .git directory");
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}/.git/worktrees/branch-xyz\n", repo.display()),
        )
        .expect("write the worktree's .git");

        let title = project_title("CC:", worktree.to_str(), "CLAUDE");
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(title, "CC:MY-PROJECT", "a worktree should show the main repo name");
    }

    #[test]
    fn title_stays_within_the_display_limit() {
        let long = project_title("CC:", Some("/work/a-very-long-project-name-here"), "CLAUDE");
        assert!(
            long.chars().count() <= MAX_TITLE_CHARS,
            "title must not exceed the display limit"
        );
        assert!(long.starts_with("CC:"));
    }
}
