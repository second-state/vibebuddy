//! The on-duty screen as the computer's lines leave it: state, task cards, today's stats, and which line to play.
//! It reads the box's state events the way the box does (`firmware-rs/core/src/firmware.rs`, `show_event`).

use serde_json::Value;

/// How many task cards the screen has room for.
pub const MAX_TASKS: usize = 3;
/// After `task.done` the screen goes back to idle by itself, as the box's does.
pub const DONE_TO_IDLE_MS: u64 = 5000;
/// How long a greeting (`buddy.say`) stays up.
pub const GREETING_MS: u64 = 8000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Working,
    InputRequired,
    Done,
    Failed,
}

/// The Agents that have an icon, by the `agent` the daemon gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
    OpenCode,
    Copilot,
    Pi,
    Ci,
}

impl Agent {
    pub fn from_name(agent: &str) -> Option<Self> {
        Some(match agent {
            "claude-code" => Self::ClaudeCode,
            "codex" => Self::Codex,
            "opencode" => Self::OpenCode,
            "copilot" => Self::Copilot,
            "pi" => Self::Pi,
            "github-actions" => Self::Ci,
            _ => return None,
        })
    }

    /// The prefix the daemon gives this Agent's titles, which the icon replaces.
    fn prefix(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CC:",
            Self::Codex => "CX:",
            Self::OpenCode => "OC:",
            Self::Copilot => "CP:",
            Self::Pi => "PI:",
            Self::Ci => "CI:",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// The id `focus` sends back; only a short one of letters and digits is kept, as on the box.
    pub id: Option<String>,
    pub agent: Option<Agent>,
    pub title: String,
    pub waiting: bool,
    pub project: Option<String>,
    pub elapsed_s: u64,
}

/// The robot's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    InputRequired,
    Done,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct Duty {
    pub state: State,
    pub tasks: Vec<Task>,
    /// All tasks, also the ones without a card.
    pub tasks_total: usize,
    /// Lines like "7 DONE", "3 ASKS", "4H 12M BUSY".
    pub stats: Vec<String>,
    /// The task the last state event is about (its `title`): the card on the full-screen states.
    pub headline: Option<Task>,
    /// The occasion of the last greeting, while it is up.
    pub greeting: Option<String>,
    /// A box is linked to the computer, so it speaks and the VibeGame doesn't.
    pub quiet: bool,
    greeting_until: u64,
    done_until: Option<u64>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The agent a title's prefix names, for a title that comes without its card.
fn agent_of(title: &str) -> Option<Agent> {
    [Agent::ClaudeCode, Agent::Codex, Agent::OpenCode, Agent::Copilot, Agent::Pi, Agent::Ci].into_iter().find(|a| title.starts_with(a.prefix()))
}

fn task(item: &Value) -> Option<Task> {
    let mut title = text(item, "title")?;
    let status = item.get("status").and_then(Value::as_str)?;
    let agent = item.get("agent").and_then(Value::as_str).and_then(Agent::from_name);
    if let Some(rest) = agent.and_then(|agent| title.strip_prefix(agent.prefix())) {
        title = rest.trim_start().to_owned();
    }
    // The project goes on the second line unless it says the same as the title (protocol.md).
    let project = text(item, "project").filter(|project| !project.eq_ignore_ascii_case(&title));
    Some(Task {
        id: text(item, "id").filter(|id| id.len() <= 16 && id.bytes().all(|b| b.is_ascii_alphanumeric())),
        agent,
        title,
        waiting: status == "input_required",
        project,
        elapsed_s: item.get("elapsed_s").and_then(Value::as_u64).unwrap_or(0),
    })
}

impl Duty {
    /// One line from the computer. Returns the line to play, if any; `quiet` means none.
    pub fn apply(&mut self, event: &Value, now: u64) -> Option<Sound> {
        if let Some(quiet) = event.get("quiet").and_then(Value::as_bool) {
            self.quiet = quiet;
        }
        let (state, mut sound) = match event.get("event").and_then(Value::as_str)? {
            "task.start" => (State::Working, None),
            "agent.idle" => (State::Idle, None),
            "agent.input_required" => (State::InputRequired, Some(Sound::InputRequired)),
            "task.done" => (State::Done, Some(Sound::Done)),
            "task.error" | "agent.blocked" => (State::Failed, Some(Sound::Failed)),
            "buddy.say" => {
                self.greeting = text(event, "occasion");
                self.greeting_until = now + GREETING_MS;
                return None;
            }
            _ => return None,
        };
        self.state = state;
        self.done_until = (state == State::Done).then_some(now + DONE_TO_IDLE_MS);
        let title = text(event, "title");
        if let Some(Value::Array(items)) = event.get("tasks") {
            self.headline = items.iter().filter(|item| title.is_some() && item.get("title").and_then(Value::as_str) == title.as_deref()).find_map(task);
            self.tasks = items.iter().filter_map(task).take(MAX_TASKS).collect();
        } else {
            self.headline = None;
        }
        if self.headline.is_none()
            && let Some(title) = title
        {
            let agent = agent_of(&title);
            let title = agent.and_then(|a| title.strip_prefix(a.prefix())).map_or(title.clone(), |t| t.trim_start().to_owned());
            self.headline = Some(Task { id: None, agent, title, waiting: state == State::InputRequired, project: None, elapsed_s: 0 });
        }
        self.tasks_total = event.get("tasks_total").and_then(Value::as_u64).map_or(self.tasks.len(), |n| n as usize);
        if let Some(Value::Array(lines)) = event.get("stats") {
            self.stats = lines.iter().filter_map(Value::as_str).map(str::to_owned).take(4).collect();
        }
        if event.get("suppress_audio") == Some(&Value::Bool(true)) {
            sound = None;
        }
        match event.get("announcement").and_then(Value::as_str) {
            Some("done") => sound = Some(Sound::Done),
            Some("failed") => sound = Some(Sound::Failed),
            _ => {}
        }
        if self.quiet { None } else { sound }
    }

    /// Done goes back to idle, and a greeting goes away, by themselves.
    pub fn tick(&mut self, now: u64) {
        if self.done_until.is_some_and(|until| now >= until) {
            self.state = State::Idle;
            self.done_until = None;
        }
        if self.greeting.is_some() && now >= self.greeting_until {
            self.greeting = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn waiting() -> Value {
        json!({"version":1,"event":"agent.input_required","tasks_total":4,"stats":["2 DONE","1 ASKS","1H 5M BUSY"],
            "tasks":[{"id":"t1","agent":"claude-code","title":"CC:Add Chinese","status":"input_required","project":"vibebuddy","elapsed_s":75},
                     {"id":"t2","agent":"codex","title":"CX:Fix voice","status":"working","elapsed_s":5},
                     {"id":"bad id!","agent":"someone","title":"Other","status":"working"},
                     {"id":"t4","title":"Fourth","status":"working"}],
            "quiet":false})
    }

    #[test]
    fn a_state_event_fills_the_cards() {
        let mut duty = Duty::default();
        assert_eq!(duty.apply(&waiting(), 0), Some(Sound::InputRequired));
        assert_eq!(duty.state, State::InputRequired);
        assert_eq!(duty.tasks.len(), MAX_TASKS);
        assert_eq!(duty.tasks_total, 4);
        let first = &duty.tasks[0];
        assert_eq!((first.id.as_deref(), first.agent, first.title.as_str(), first.waiting), (Some("t1"), Some(Agent::ClaudeCode), "Add Chinese", true));
        assert_eq!(first.project.as_deref(), Some("vibebuddy"));
        assert_eq!(duty.tasks[2].id, None, "an id that can't go back out as is");
        assert_eq!(duty.tasks[2].agent, None);
        assert_eq!(duty.stats, ["2 DONE", "1 ASKS", "1H 5M BUSY"]);
    }

    #[test]
    fn the_title_picks_the_headline_card_and_a_project_like_the_title_is_left_out() {
        let mut duty = Duty::default();
        duty.apply(&json!({"event":"task.done","title":"WEBSITE","tasks":[
            {"id":"a1","agent":"claude-code","title":"VIBEBUDDY","project":"vibebuddy","status":"working"},
            {"id":"a2","agent":"claude-code","title":"WEBSITE","project":"website","status":"done"}]}), 0);
        let headline = duty.headline.clone().unwrap();
        assert_eq!((headline.id.as_deref(), headline.project), (Some("a2"), None));
        duty.apply(&json!({"event":"task.error","title":"CI:App tests","tasks":[]}), 0);
        let headline = duty.headline.clone().unwrap();
        assert_eq!((headline.agent, headline.title.as_str()), (Some(Agent::Ci), "App tests"));
        duty.apply(&json!({"event":"agent.idle"}), 0);
        assert_eq!(duty.headline, None);
    }

    #[test]
    fn quiet_shows_everything_and_plays_nothing() {
        let mut duty = Duty::default();
        let mut event = waiting();
        event["quiet"] = json!(true);
        assert_eq!(duty.apply(&event, 0), None);
        assert!(duty.quiet);
        assert_eq!(duty.state, State::InputRequired);
        assert_eq!(duty.tasks.len(), MAX_TASKS);
        assert_eq!(duty.apply(&json!({"event":"task.done","announcement":"done","quiet":true}), 0), None);
        assert_eq!(duty.apply(&json!({"event":"task.done","quiet":false}), 0), Some(Sound::Done));
    }

    #[test]
    fn suppress_audio_redraws_without_a_line_unless_an_announcement_says_otherwise() {
        let mut duty = Duty::default();
        let mut event = waiting();
        event["suppress_audio"] = json!(true);
        assert_eq!(duty.apply(&event, 0), None);
        event["announcement"] = json!("failed");
        assert_eq!(duty.apply(&event, 0), Some(Sound::Failed));
        assert_eq!(duty.apply(&json!({"event":"agent.blocked"}), 0), Some(Sound::Failed));
        assert_eq!(duty.apply(&json!({"event":"task.start"}), 0), None);
    }

    #[test]
    fn done_goes_back_to_idle_after_five_seconds() {
        let mut duty = Duty::default();
        duty.apply(&json!({"event":"task.done","tasks":[]}), 1000);
        duty.tick(1000 + DONE_TO_IDLE_MS - 1);
        assert_eq!(duty.state, State::Done);
        duty.tick(1000 + DONE_TO_IDLE_MS);
        assert_eq!(duty.state, State::Idle);
    }

    #[test]
    fn a_greeting_shows_for_a_while_and_other_events_are_ignored() {
        let mut duty = Duty::default();
        assert_eq!(duty.apply(&json!({"event":"buddy.say","occasion":"greeting_morning","quiet":false}), 0), None);
        assert_eq!(duty.greeting.as_deref(), Some("greeting_morning"));
        duty.tick(GREETING_MS);
        assert_eq!(duty.greeting, None);
        assert_eq!(duty.apply(&json!({"event":"device.heartbeat","build":"x"}), 0), None);
        assert_eq!(duty.apply(&json!({"event":"voice.begin"}), 0), None);
        assert_eq!(duty.state, State::Idle);
    }
}
