//! Maps Claude Code lifecycle events onto the generic activity model.
//!
//! Shares the aggregator with the Codex adapter; only event names and how activity identity is built differ.

use vibebuddy_protocol::Event;
use serde::Deserialize;

use crate::activity::{
    ActivityId, ActivitySource, ActivityStatus, ActivityTracker, Surface, TmuxPane, card_title,
    display_title, project_name,
};
use crate::session_titles::{SessionTitles, git_branch};

/// Prefix on task cards that tells agents apart.
const PREFIX: &str = "CC:";
/// Task-card title when the working directory isn't available.
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
    #[serde(default)]
    pub desktop_session_id: Option<String>,
}

pub fn apply(
    tracker: &mut ActivityTracker,
    titles: &mut SessionTitles,
    hook: ClaudeHook,
) -> Option<Event> {
    tracker.note_workspace(hook.cwd.as_deref());
    let id = activity_id(&hook);
    let cwd = hook.cwd.as_deref();
    // First line: the title Claude App gave the session, else the branch, else the project name.
    let project = project_name(cwd);
    let candidates = [titles.claude(&hook.session_id, cwd), cwd.and_then(git_branch)];
    let title = card_title(PREFIX, &candidates, project.as_deref(), FALLBACK_TITLE);
    tracker.note_project(&id, &display_title("", project.as_deref().unwrap_or(FALLBACK_TITLE), FALLBACK_TITLE));
    let source = ActivitySource::ClaudeCode {
        session_id: hook.session_id.clone(),
        cwd: hook.cwd.clone(),
        surface: Surface::from_hook(
            hook.surface.as_deref(),
            hook.host_bundle_id.clone(),
            hook.host_tty.clone(),
            TmuxPane::from_hook(hook.tmux_socket.clone(), hook.tmux_pane.clone()),
            hook.host_pids.clone(),
            hook.desktop_session_id.clone(),
        ),
    };
    let event = match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => {
            tracker.clear_turns(&hook.session_id);
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "PermissionRequest" => tracker.require_input(&id, &title),
        "PostToolUse" | "SubagentStart" if hook.agent_id.is_some() => tracker.observe_child(&id, &title),
        "PostToolUse" => tracker.observe(&id, &title, ActivityStatus::Working),
        "Stop" => {
            if hook.response_kind.as_deref() == Some("input_required") {
                tracker.require_input(&id, &title)
            } else {
                tracker.finish(&id, &title)
            }
        }
        // A subagent reports to its parent session, not to the user: drop its card without announcing.
        "SubagentStop" => tracker.discard(&id, "ALL QUIET"),
        // The turn ended on an API error: neither a success nor a task failure.
        "StopFailure" => tracker.discard(&id, "STOPPED"),
        "SessionEnd" => tracker.discard_session(&hook.session_id, "ALL QUIET"),
        _ => None,
    };
    tracker.associate_source(&id, source);
    event
}

/// Claude Code's background agents share the parent session's `session_id` and `prompt_id`,
/// so `agent_id` must be part of the identity, or parallel subagents overwrite each other.
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
            surface: None,
            host_bundle_id: None,
            host_tty: None,
            tmux_socket: None,
            tmux_pane: None,
            host_pids: None,
            desktop_session_id: None,
        }
    }

    fn subagent_hook(name: &str, agent_id: &str) -> ClaudeHook {
        ClaudeHook {
            agent_id: Some(agent_id.to_owned()),
            ..hook(name, "/work/vibe-buddy")
        }
    }

    #[test]
    fn maps_claude_lifecycle_without_prompt_content() {
        let mut tracker = ActivityTracker::default();

        let working = apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"))
            .expect("start event should be visible");
        assert_eq!(working.event, "task.start");
        assert_eq!(working.title.as_deref(), Some("CC:VIBE-BUDDY"));

        let waiting = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("PermissionRequest", "/work/vibe-buddy"),
        )
        .expect("permission request should be visible");
        assert_eq!(waiting.event, "agent.input_required");

        let done = apply(&mut tracker, &mut SessionTitles::disabled(), hook("Stop", "/work/vibe-buddy")).expect("stop event should be visible");
        assert_eq!(done.event, "task.done");
    }

    #[test]
    fn parallel_subagents_are_separate_activities() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"));
        apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStart", "agent-1"));
        let two = apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStart", "agent-2"))
            .expect("second subagent should refresh the card stack");

        let tasks = two.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 3, "parent session and two subagents should each get a card");
    }

    #[test]
    fn a_subagent_finishing_is_not_announced() {
        // Its result goes to the parent session, not to the user: there is nothing to come back for yet.
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"));
        apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStart", "agent-1"));

        let stopped = apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStop", "agent-1"))
            .expect("the subagent's card should leave the stack");
        assert_eq!(stopped.event, "task.start", "the parent session is still working");
        assert!(!stopped.extra.contains_key("announcement"));
        assert_eq!(stopped.extra["tasks"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn work_with_background_subagents_is_announced_once_when_it_is_all_done() {
        // Recorded on 2026-10-05: one request with two background reviewers used to say "done" five times.
        let mut tracker = ActivityTracker::default();
        let mut events = Vec::new();
        let turn = |name: &str, prompt: &str| ClaudeHook {
            prompt_id: Some(prompt.to_owned()),
            ..hook(name, "/work/vibe-buddy")
        };
        let send = |tracker: &mut ActivityTracker, events: &mut Vec<Event>, hook: ClaudeHook| {
            if let Some(event) = apply(tracker, &mut SessionTitles::disabled(), hook) {
                events.push(event);
            }
        };

        send(&mut tracker, &mut events, turn("UserPromptSubmit", "ask"));
        send(&mut tracker, &mut events, subagent_hook("SubagentStart", "reviewer-1"));
        send(&mut tracker, &mut events, subagent_hook("SubagentStart", "reviewer-2"));
        // The parent ends its turn to wait for them.
        send(&mut tracker, &mut events, turn("Stop", "ask"));
        // An injected message (a CI event) wakes it and it ends again right away.
        send(&mut tracker, &mut events, turn("UserPromptSubmit", "ci-event"));
        send(&mut tracker, &mut events, turn("Stop", "ci-event"));
        // The first reviewer finishes; its notification wakes the parent, which says it is still waiting.
        send(&mut tracker, &mut events, subagent_hook("PostToolUse", "reviewer-2"));
        send(&mut tracker, &mut events, subagent_hook("SubagentStop", "reviewer-1"));
        send(&mut tracker, &mut events, turn("UserPromptSubmit", "notification-1"));
        send(&mut tracker, &mut events, turn("Stop", "notification-1"));
        let announced: Vec<_> = events.iter().filter(|event| event.extra.contains_key("announcement")).collect();
        assert!(announced.is_empty(), "nothing is done while a reviewer still runs: {announced:?}");
        assert_eq!(events.last().map(|event| event.event.as_str()), Some("task.start"));

        // The second reviewer finishes, and the parent wraps up the work.
        send(&mut tracker, &mut events, subagent_hook("SubagentStop", "reviewer-2"));
        send(&mut tracker, &mut events, turn("UserPromptSubmit", "notification-2"));
        send(&mut tracker, &mut events, turn("PostToolUse", "notification-2"));
        send(&mut tracker, &mut events, turn("Stop", "notification-2"));
        let announced: Vec<_> = events.iter().filter(|event| event.extra.contains_key("announcement")).collect();
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].event, "task.done");
    }

    #[test]
    fn a_new_prompt_keeps_the_cards_of_subagents_still_running() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"));
        apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStart", "agent-1"));
        let next = ClaudeHook {
            prompt_id: Some("turn-b".to_owned()),
            ..hook("UserPromptSubmit", "/work/vibe-buddy")
        };
        apply(&mut tracker, &mut SessionTitles::disabled(), next);
        let stop = ClaudeHook {
            prompt_id: Some("turn-b".to_owned()),
            ..hook("Stop", "/work/vibe-buddy")
        };
        let stopped = apply(&mut tracker, &mut SessionTitles::disabled(), stop).expect("the stack should refresh");
        assert!(!stopped.extra.contains_key("announcement"), "the subagent from the earlier turn still runs");
        assert_eq!(stopped.extra["tasks"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn stop_that_waits_for_a_reply_requests_input_instead_of_reporting_done() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"));
        let stop: ClaudeHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "prompt_id": "turn-a",
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
    fn a_new_turn_replaces_the_previous_one() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/alpha"));
        let next = ClaudeHook {
            prompt_id: Some("turn-b".to_owned()),
            ..hook("UserPromptSubmit", "/work/beta")
        };
        let resumed = apply(&mut tracker, &mut SessionTitles::disabled(), next).expect("new turn should be visible");

        let tasks = resumed.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 1, "the previous turn of the same session should be cleared");
        assert_eq!(tasks[0]["title"], "CC:BETA");
    }

    #[test]
    fn api_failure_does_not_report_success() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/alpha"));

        let stopped = apply(&mut tracker, &mut SessionTitles::disabled(), hook("StopFailure", "/work/alpha"))
            .expect("API error should return to idle");
        assert_eq!(stopped.event, "agent.idle");
        assert_eq!(stopped.title.as_deref(), Some("STOPPED"));
    }

    #[test]
    fn both_agents_share_one_card_stack() {
        use crate::codex_hooks::{self, CodexHook};

        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("UserPromptSubmit", "/work/vibe-buddy"));
        let codex = CodexHook {
            session_id: "codex-session".to_owned(),
            turn_id: Some("codex-turn".to_owned()),
            thread_id: None,
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
        let mixed = codex_hooks::apply(&mut tracker, &mut SessionTitles::disabled(), codex).expect("another agent should refresh the card stack");

        let tasks = mixed.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 2, "two agents in the same directory should each get a card");
        assert_eq!(tasks[0]["title"], "CX:VIBE-BUDDY");
        assert_eq!(tasks[1]["title"], "CC:VIBE-BUDDY");
    }
}
