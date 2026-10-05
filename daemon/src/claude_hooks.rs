//! Maps Claude Code lifecycle events onto the generic activity model.
//!
//! Shares the aggregator with the Codex adapter; only event names and how activity identity is built differ.

use vibebuddy_protocol::Event;
use serde::Deserialize;

use crate::activity::{
    ActivityId, ActivitySource, ActivityStatus, ActivityTracker, Surface, card_title,
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
            hook.host_pids.clone(),
            hook.desktop_session_id.clone(),
        ),
    };
    let event = match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => {
            tracker.clear_session(&hook.session_id);
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "PermissionRequest" => tracker.require_input(&id, &title),
        "PostToolUse" | "SubagentStart" => tracker.observe(&id, &title, ActivityStatus::Working),
        "Stop" => {
            if hook.response_kind.as_deref() == Some("input_required") {
                tracker.require_input(&id, &title)
            } else {
                tracker.finish(&id, &title)
            }
        }
        "SubagentStop" => tracker.finish(&id, &title),
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

        let first = apply(&mut tracker, &mut SessionTitles::disabled(), subagent_hook("SubagentStop", "agent-1"))
            .expect("subagent stop should produce an event");
        assert_eq!(
            first.extra.get("announcement").and_then(|v| v.as_str()),
            Some("done"),
            "one subagent stopping must not swallow the announcement"
        );
        let remaining = first.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(remaining.len(), 2);
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
            host_pids: None,
        };
        let mixed = codex_hooks::apply(&mut tracker, &mut SessionTitles::disabled(), codex).expect("another agent should refresh the card stack");

        let tasks = mixed.extra["tasks"].as_array().expect("tasks should be an array");
        assert_eq!(tasks.len(), 2, "two agents in the same directory should each get a card");
        assert_eq!(tasks[0]["title"], "CX:VIBE-BUDDY");
        assert_eq!(tasks[1]["title"], "CC:VIBE-BUDDY");
    }
}
