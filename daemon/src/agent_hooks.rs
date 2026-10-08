//! Maps the agents without a desktop app of their own (OpenCode, GitHub Copilot CLI) onto the generic activity
//! model. Their hook already turned each agent's events into the same five, so one adapter serves them all.

use serde::Deserialize;
use vibebuddy_protocol::Event;

use crate::activity::{
    ActivityId, ActivitySource, ActivityStatus, ActivityTracker, Surface, TmuxPane, card_title, display_title,
    project_name,
};
use crate::session_titles::git_branch;

#[derive(Debug, Deserialize)]
pub struct AgentHook {
    pub agent: String,
    pub session_id: String,
    /// The turn, when the agent names it (OpenCode's prompt message); otherwise a turn is the session's latest.
    #[serde(default)]
    pub turn_id: Option<String>,
    pub event: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub response_kind: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub host_bundle_id: Option<String>,
    #[serde(default)]
    pub host_ttys: Option<Vec<String>>,
    #[serde(default)]
    pub tmux_socket: Option<String>,
    #[serde(default)]
    pub tmux_pane: Option<String>,
    #[serde(default)]
    pub host_focus_url: Option<String>,
    #[serde(default)]
    pub host_pids: Option<Vec<u32>>,
}

/// The task card prefix and the title used without a working directory, per agent. Anything else is refused,
/// so the box never shows a name a local process made up.
fn card_names(agent: &str) -> Option<(&'static str, &'static str)> {
    match agent {
        "opencode" => Some(("OC:", "OPENCODE")),
        "copilot" => Some(("CP:", "COPILOT")),
        _ => None,
    }
}

pub fn apply(tracker: &mut ActivityTracker, hook: AgentHook) -> Option<Event> {
    let (prefix, fallback) = card_names(&hook.agent)?;
    tracker.note_workspace(hook.cwd.as_deref());
    let id = activity_id(&hook);
    let cwd = hook.cwd.as_deref();
    let project = project_name(cwd);
    let title = card_title(prefix, &[cwd.and_then(git_branch)], project.as_deref(), fallback);
    tracker.note_project(&id, &display_title("", project.as_deref().unwrap_or(fallback), fallback));
    let source = ActivitySource::Cli {
        agent: hook.agent.clone(),
        session_id: hook.session_id.clone(),
        surface: Surface::from_hook(
            hook.surface.as_deref(),
            hook.host_bundle_id.clone(),
            hook.host_ttys.clone(),
            TmuxPane::from_hook(hook.tmux_socket.clone(), hook.tmux_pane.clone()),
            hook.host_focus_url.clone(),
            hook.host_pids.clone(),
            None,
        ),
    };
    let event = match hook.event.as_str() {
        "working" => {
            // A new turn replaces the previous one; repeats within the turn only keep it alive.
            if !tracker.has_activity(&id) {
                tracker.clear_turns(&hook.session_id);
            }
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "needs_input" => tracker.require_input(&id, &title),
        "done" if hook.response_kind.as_deref() == Some("input_required") => tracker.require_input(&id, &title),
        "done" => tracker.finish(&id, &title),
        // Interrupted or failed: neither a success nor something to wait on.
        "stopped" => tracker.discard(&id, "STOPPED"),
        "session_end" => tracker.discard_session(&hook.session_id, "ALL QUIET"),
        _ => None,
    };
    tracker.associate_source(&id, source);
    event
}

fn activity_id(hook: &AgentHook) -> ActivityId {
    let session_id = format!("{}:{}", hook.agent, hook.session_id);
    let key = match hook.turn_id.as_deref() {
        Some(turn) => format!("{session_id}:{turn}"),
        None => session_id.clone(),
    };
    ActivityId { session_id, key }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(agent: &str, event: &str, turn: Option<&str>) -> AgentHook {
        serde_json::from_value(serde_json::json!({
            "agent": agent, "session_id": "ses_1", "turn_id": turn, "event": event, "cwd": "/work/vibe-buddy",
        }))
        .expect("hook")
    }

    #[test]
    fn a_turn_is_announced_once_when_it_ends() {
        let mut tracker = ActivityTracker::default();
        assert!(apply(&mut tracker, hook("opencode", "working", Some("msg_1"))).is_some());
        apply(&mut tracker, hook("opencode", "working", Some("msg_1")));
        let done = apply(&mut tracker, hook("opencode", "done", Some("msg_1"))).expect("announced");
        assert_eq!(done.event, "task.done");
        // OpenCode reports idle again without a new prompt: nothing more to say.
        assert!(apply(&mut tracker, hook("opencode", "done", Some("msg_1"))).is_none_or(|event| event.event != "task.done"));
    }

    #[test]
    fn a_question_at_the_end_of_the_turn_waits_on_the_user() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("copilot", "working", None));
        let mut asking = hook("copilot", "done", None);
        asking.response_kind = Some("input_required".to_owned());
        let event = apply(&mut tracker, asking).expect("event");
        assert_ne!(event.event, "task.done");
    }

    #[test]
    fn an_interrupted_turn_is_not_announced_as_done() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, hook("opencode", "working", Some("msg_1")));
        apply(&mut tracker, hook("opencode", "stopped", Some("msg_1")));
        let after = apply(&mut tracker, hook("opencode", "done", Some("msg_1")));
        assert!(after.is_none_or(|event| event.event != "task.done"));
    }

    #[test]
    fn the_source_is_the_agents_session_in_its_terminal() {
        let mut tracker = ActivityTracker::default();
        let mut working = hook("opencode", "working", Some("msg_1"));
        working.surface = Some("host".to_owned());
        working.host_bundle_id = Some("com.mitchellh.ghostty".to_owned());
        working.host_ttys = Some(vec!["ttys016".to_owned()]);
        apply(&mut tracker, working);
        assert_eq!(
            tracker.focus_source(),
            Some(ActivitySource::Cli {
                agent: "opencode".to_owned(),
                session_id: "ses_1".to_owned(),
                surface: Surface::Host {
                    bundle_id: "com.mitchellh.ghostty".to_owned(),
                    ttys: vec!["ttys016".to_owned()],
                    tmux: None,
                    focus_url: None,
                },
            })
        );
    }

    #[test]
    fn an_unknown_agent_is_refused() {
        let mut tracker = ActivityTracker::default();
        assert!(apply(&mut tracker, hook("made-up", "working", None)).is_none());
        assert_eq!(tracker.focus_source(), None);
    }
}
