//! Maps Codex lifecycle events onto the generic activity model.
//!
//! This only translates: task-card aggregation, dedup, announcements and expiry live in [`crate::activity`].
//! The adapter holds no state, because the device has one screen and all agents share one aggregator.

use vibebuddy_protocol::Event;
use serde::Deserialize;

use crate::activity::{
    ActivityId, ActivitySource, ActivityStatus, ActivityTracker, card_title, display_title,
    project_name, Surface, TmuxPane};
use crate::session_titles::{SessionTitles, git_branch};

/// Prefix on task cards that tells agents apart.
const PREFIX: &str = "CX:";
/// Task-card title when the working directory isn't available.
const FALLBACK_TITLE: &str = "CODEX";

#[derive(Debug, Deserialize)]
pub struct CodexHook {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub response_kind: Option<String>,
    /// Where it runs: the hook decides from the process environment on this Mac; the daemon only routes.
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub host_bundle_id: Option<String>,
    #[serde(default)]
    pub host_tty: Option<String>,
    #[serde(default)]
    pub tmux_socket: Option<String>,
    #[serde(default)]
    pub tmux_pane: Option<String>,
    #[serde(default)]
    pub host_pids: Option<Vec<u32>>,
}

pub fn apply(
    tracker: &mut ActivityTracker,
    titles: &mut SessionTitles,
    hook: CodexHook,
) -> Option<Event> {
    tracker.note_workspace(hook.cwd.as_deref());
    let id = activity_id(&hook);
    let cwd = hook.cwd.as_deref();
    // First line: the Codex thread's name (user-given, or the branch the thread recorded), else
    // the local branch, else the project name.
    let project = project_name(cwd);
    let thread_id = hook.thread_id.clone().unwrap_or_else(|| hook.session_id.clone());
    // Codex's background sessions (the ones that generate ambient suggestions after a turn) fire
    // the same hooks: no working directory, and absent from the thread table. They aren't user activity:
    // no card, no announcement, and certainly not a K2 target, since opening one gives a blank session.
    if project.is_none() && titles.codex_thread_known(&thread_id) == Some(false) {
        tracing::info!(
            session = %hook.session_id,
            event = %hook.hook_event_name,
            "ignoring Codex background session without a thread"
        );
        return None;
    }
    let candidates = [titles.codex(&thread_id), cwd.and_then(git_branch)];
    let title = card_title(PREFIX, &candidates, project.as_deref(), FALLBACK_TITLE);
    tracker.note_project(&id, &display_title("", project.as_deref().unwrap_or(FALLBACK_TITLE), FALLBACK_TITLE));
    let source = ActivitySource::Codex {
        thread_id: hook
            .thread_id
            .clone()
            .unwrap_or_else(|| hook.session_id.clone()),
        surface: Surface::from_hook(
            hook.surface.as_deref(),
            hook.host_bundle_id.clone(),
            hook.host_tty.clone(),
            TmuxPane::from_hook(hook.tmux_socket.clone(), hook.tmux_pane.clone()),
            hook.host_pids.clone(),
            None,
        ),
    };
    let event = match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => {
            tracker.clear_session(&hook.session_id);
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "PermissionRequest" => tracker.require_input(&id, &title),
        "PostToolUse" => tracker.observe(&id, &title, ActivityStatus::Working),
        "Stop" => {
            if hook.response_kind.as_deref() == Some("input_required") {
                tracker.require_input(&id, &title)
            } else {
                tracker.finish(&id, &title)
            }
        }
        "Interrupt" => tracker.discard(&id, "INTERRUPTED"),
        "SessionEnd" => tracker.discard_session(&hook.session_id, "ALL QUIET"),
        _ => None,
    };
    tracker.associate_source(&id, source);
    event
}

/// Codex builds activity identity from `session_id` and `turn_id`.
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

    #[test]
    fn background_sessions_without_a_thread_are_ignored() {
        let dir = std::env::temp_dir().join(format!("codex-ghost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.sqlite");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, git_branch TEXT);
                 INSERT INTO threads VALUES ('real-thread', 'Review', NULL);",
            )
            .unwrap();
        let mut titles = SessionTitles::with_codex_db(db);
        let mut tracker = ActivityTracker::default();

        let ghost = CodexHook {
            session_id: "ghost".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("ghost".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: None,
            response_kind: None,
            surface: None,
            host_bundle_id: None,
            host_tty: None,
            tmux_socket: None,
            tmux_pane: None,
            host_pids: None,
        };
        assert!(apply(&mut tracker, &mut titles, ghost).is_none());
        assert!(tracker.focus_source().is_none());

        // Sessions with a working directory count as usual, even if the thread table doesn't have them yet.
        let fresh = CodexHook {
            session_id: "fresh".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("fresh".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            response_kind: None,
            surface: None,
            host_bundle_id: None,
            host_tty: None,
            tmux_socket: None,
            tmux_pane: None,
            host_pids: None,
        };
        assert!(apply(&mut tracker, &mut titles, fresh).is_some());

        // Sessions in the thread table count even without a working directory.
        let known = CodexHook {
            session_id: "real-thread".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("real-thread".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: None,
            response_kind: None,
            surface: None,
            host_bundle_id: None,
            host_tty: None,
            tmux_socket: None,
            tmux_pane: None,
            host_pids: None,
        };
        let event = apply(&mut tracker, &mut titles, known).expect("known thread should be visible");
        assert_eq!(event.title.as_deref(), Some("CX:REVIEW"));
    }

    fn hook(session: &str, name: &str, cwd: &str) -> CodexHook {
        hook_with_turn(session, &format!("{session}-turn"), name, cwd)
    }

    fn hook_with_turn(session: &str, turn: &str, name: &str, cwd: &str) -> CodexHook {
        CodexHook {
            session_id: session.to_owned(),
            turn_id: Some(turn.to_owned()),
            thread_id: None,
            hook_event_name: name.to_owned(),
            cwd: Some(cwd.to_owned()),
            response_kind: None,
            surface: None,
            host_bundle_id: None,
            host_tty: None,
            tmux_socket: None,
            tmux_pane: None,
            host_pids: None,
        }
    }

    #[test]
    fn codex_source_uses_explicit_navigable_thread_id() {
        let mut tracker = ActivityTracker::default();
        let hook: CodexHook = serde_json::from_value(json!({
            "session_id": "child-session",
            "turn_id": "child-turn",
            "thread_id": "parent-thread",
            "hook_event_name": "UserPromptSubmit",
            "cwd": "/work/memories"
        }))
        .expect("hook with a navigable thread should parse");

        apply(&mut tracker, &mut SessionTitles::disabled(), hook);

        assert_eq!(
            tracker.focus_source(),
            Some(ActivitySource::Codex {
                thread_id: "parent-thread".to_owned(),
                surface: Surface::default(),
            })
        );
    }

    #[test]
    fn maps_codex_lifecycle_without_prompt_content() {
        let mut tracker = ActivityTracker::default();

        let working = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/vibe-buddy"),
        )
        .expect("start event should be visible");
        assert_eq!(working.event, "task.start");
        assert_eq!(working.title.as_deref(), Some("CX:VIBE-BUDDY"));

        let waiting = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "PermissionRequest", "/work/vibe-buddy"),
        )
        .expect("approval event should be visible");
        assert_eq!(waiting.event, "agent.input_required");

        let resumed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "PostToolUse", "/work/vibe-buddy"),
        )
        .expect("should return to working after the tool finishes");
        assert_eq!(resumed.event, "task.start");

        let done = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "Stop", "/work/vibe-buddy"),
        )
        .expect("stop event should be visible");
        assert_eq!(done.event, "task.done");
    }

    #[test]
    fn stop_that_waits_for_a_reply_requests_input_instead_of_reporting_done() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/vibe-buddy"),
        );
        let stop: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "session-a-turn",
            "hook_event_name": "Stop",
            "cwd": "/work/vibe-buddy",
            "response_kind": "input_required"
        }))
        .expect("Stop payload awaiting an answer should parse");

        let waiting = apply(&mut tracker, &mut SessionTitles::disabled(), stop).expect("awaiting an answer should produce a visible event");
        assert_eq!(waiting.event, "agent.input_required");
        assert!(!waiting.extra.contains_key("announcement"));
    }

    #[test]
    fn input_required_has_priority_over_other_work() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("working", "UserPromptSubmit", "/work/alpha"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/beta"),
        );

        let waiting = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/beta"),
        )
        .expect("input required should become a visible state");
        assert_eq!(waiting.event, "agent.input_required");
        assert_eq!(waiting.title.as_deref(), Some("CX:BETA"));

        // The finish announcement says who finished, so the screen shows that one; alpha is still running and
        // its later events will bring the screen back.
        let done =
            apply(&mut tracker, &mut SessionTitles::disabled(), hook("waiting", "Stop", "/work/beta")).expect("stop should produce an announcement");
        assert_eq!(done.event, "task.done");
        assert_eq!(done.title.as_deref(), Some("CX:BETA"));

        let back = apply(&mut tracker, &mut SessionTitles::disabled(), hook("working", "PostToolUse", "/work/alpha"))
            .expect("the next event should hand the screen back to the still-running task");
        assert_eq!(back.event, "task.start");
        assert_eq!(back.title.as_deref(), Some("CX:ALPHA"));
    }

    /// One task finishes while another is waiting for an answer: the screen must stay on the waiting one,
    /// since it needs the user to act; a single "done" announcement is enough.
    #[test]
    fn finishing_one_task_does_not_hide_another_waiting_for_a_reply() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("finishing", "UserPromptSubmit", "/work/alpha"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/beta"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/beta"),
        );

        let done =
            apply(&mut tracker, &mut SessionTitles::disabled(), hook("finishing", "Stop", "/work/alpha")).expect("stop should produce an announcement");

        assert_eq!(
            done.extra.get("announcement").and_then(|v| v.as_str()),
            Some("done"),
            "completion must still be announced"
        );
        assert_eq!(done.event, "agent.input_required");
        assert_eq!(
            done.title.as_deref(),
            Some("CX:BETA"),
            "the screen should stay on the one waiting for an answer"
        );
    }

    #[test]
    fn background_refresh_does_not_reannounce_existing_input_request() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/waiting"),
        );
        let first = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/waiting"),
        )
        .expect("first input wait should be visible");
        assert!(!first.extra.contains_key("suppress_audio"));

        let refreshed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("working", "UserPromptSubmit", "/work/working"),
        )
        .expect("background task change should refresh the cards");
        assert_eq!(refreshed.event, "agent.input_required");
        assert_eq!(refreshed.extra["suppress_audio"], true);
    }

    #[test]
    fn task_cards_are_newest_first_and_limited_to_three() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("one", "UserPromptSubmit", "/work/one"));
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("two", "UserPromptSubmit", "/work/two"));
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("three", "UserPromptSubmit", "/work/three"),
        );
        let latest = apply(&mut tracker, &mut SessionTitles::disabled(), hook("four", "UserPromptSubmit", "/work/four"))
            .expect("new task should refresh the card stack");

        let tasks = latest.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0]["title"], "CX:FOUR");
        assert_eq!(tasks[1]["title"], "CX:THREE");
        assert_eq!(tasks[2]["title"], "CX:TWO");
    }

    #[test]
    fn background_task_removal_refreshes_the_stack() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("old", "UserPromptSubmit", "/work/old"));
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("new", "UserPromptSubmit", "/work/new"));

        let refreshed = apply(&mut tracker, &mut SessionTitles::disabled(), hook("old", "Stop", "/work/old"))
            .expect("background task ending should also refresh the card stack");
        let tasks = refreshed.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "CX:NEW");
    }

    #[test]
    fn duplicate_visible_state_is_suppressed() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/alpha"),
        );

        assert!(
            apply(
                &mut tracker,
                &mut SessionTitles::disabled(),
                hook("session-a", "PostToolUse", "/work/alpha")
            )
            .is_none()
        );
    }

    #[test]
    fn reply_starts_a_new_turn_and_clears_the_previous_waiting_turn() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-a", "UserPromptSubmit", "/work/alpha"),
        );
        let waiting: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/alpha",
            "response_kind": "input_required"
        }))
        .expect("Stop payload awaiting an answer should parse");
        assert_eq!(
            apply(&mut tracker, &mut SessionTitles::disabled(), waiting).expect("a question should wait for an answer").event,
            "agent.input_required"
        );

        let resumed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-b", "UserPromptSubmit", "/work/beta"),
        )
        .expect("a new turn should start after the user answers");
        assert_eq!(resumed.event, "task.start");
        assert_eq!(resumed.title.as_deref(), Some("CX:BETA"));

        let replay = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-a", "Stop", "/work/alpha"),
        );
        assert!(replay.is_none(), "a repeated Stop from the old turn must not change the new turn");

        let second = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-b", "Stop", "/work/beta"),
        )
        .expect("the second turn should produce a completion notice");
        assert_eq!(second.event, "task.done");
    }

    #[test]
    fn interrupt_does_not_report_success() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/alpha"),
        );

        let interrupted = apply(&mut tracker, &mut SessionTitles::disabled(), hook("session-a", "Interrupt", "/work/alpha"))
            .expect("interrupt should return to idle");
        assert_eq!(interrupted.event, "agent.idle");
        assert_eq!(interrupted.title.as_deref(), Some("INTERRUPTED"));
    }

    #[test]
    fn every_tracked_stop_announces_completion_with_parallel_tasks() {
        let mut tracker = ActivityTracker::default();
        for session in ["one", "two", "three"] {
            apply(
                &mut tracker,
                &mut SessionTitles::disabled(),
                hook(session, "UserPromptSubmit", "/work/project"),
            );
        }

        let announcements = ["one", "two", "three"]
            .into_iter()
            .filter(|session| {
                let event = apply(&mut tracker, &mut SessionTitles::disabled(), hook(session, "Stop", "/work/project"))
                    .expect("every active session ending should produce an event");
                event.event == "task.done"
                    || event
                        .extra
                        .get("announcement")
                        .and_then(|value| value.as_str())
                        == Some("done")
            })
            .count();

        assert_eq!(announcements, 3, "three sessions should trigger three completion announcements");
    }
}
