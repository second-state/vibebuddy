//! The resident session (ADR-0017): the one long-running Agent session every voice command goes to, on one of two
//! brains: the user's Codex (this module) or the user's Claude Code with their own Anthropic API key
//! ([`crate::resident_claude`]). Which one is a setting (`codex`, `claude` or `auto`: Codex when it is ready, else
//! Claude Code when a key is set); each brain keeps its own session for good.
//!
//! The daemon drives `codex app-server` over stdio with the user's own login, in a Codex home of its own
//! (`resident-codex/` in the daemon's state directory) that the user's Codex and the ChatGPT App never list, so they
//! never take the thread's writer lock: `thread/start` once, `thread/resume` on every later start, `turn/start` per
//! voice command. The session answers through the `speak`
//! tool we define (`dynamicTools`), and reads the Activities through `list_activities`; it gets no tool that runs
//! commands or writes files, runs read-only, and every approval or question it raises is declined. It reads the
//! user's Codex memories, but only through the daemon, which never writes them.
//!
//! Experimental and off unless `VIBEBUDDY_RESIDENT_SESSION=1`. The app-server starts on the first voice command
//! and again on the next one after it dies. Voice commands are taken one at a time; the ones that arrive meanwhile
//! wait their turn in arrival order.
//!
//! The resident session is never an Activity: should any Codex hooks fire for its thread, [`Resident::owns_hook`]
//! is how the Codex adapter recognises and drops them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc, watch};
use tracing::{info, warn};

use crate::activity::ActivityTracker;
use crate::app_server::{Client, Incoming, RpcError};
use crate::persona::{Persona, Wearing};
use crate::resident_claude::{CONTEXT_PREFIX, ClaudeBrain};

/// The Codex version the app-server protocol was verified with. app-server and `dynamicTools` are experimental,
/// so other versions get a warning in the log, but are still tried.
pub const TESTED_CODEX_VERSION: &str = "0.162.0";
/// Setting this to `1` turns the resident session on.
pub const ENABLE_VARIABLE: &str = "VIBEBUDDY_RESIDENT_SESSION";
/// Names the `codex` binary explicitly; otherwise it is looked for on PATH and where installers put it.
pub const CODEX_VARIABLE: &str = "VIBEBUDDY_CODEX";
/// How long a single request (handshake, thread start, turn start) may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one voice command may take from `turn/start` to `turn/completed`; then the turn is interrupted.
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
/// The resident session's own Codex home, in the daemon's state directory.
const CODEX_HOME_DIR: &str = "resident-codex";
/// How much of the user's `memory_summary.md` goes into the instructions: Codex's own cap, 2,500 tokens, which it
/// counts as 4 bytes each.
const MEMORY_SUMMARY_MAX_BYTES: usize = 10_000;
/// How much one `search_memories` call returns at most.
const MEMORY_SEARCH_MAX_BYTES: usize = 8_000;
/// The model the resident session runs on, and its reasoning effort: a voice command waits on the answer, and the
/// session only talks, looks things up and hands work on. See [`tool_overrides`] for what was measured.
const MODEL: &str = "gpt-6-luna";
const REASONING_EFFORT: &str = "low";
/// How long a readiness check (is `codex` there, is the user logged in) stands before it is made again.
pub const READINESS_TTL: Duration = Duration::from_secs(30);
/// The login store the resident Codex uses: the user's `auth.json`, linked in (see [`tool_overrides`]).
const AUTH_STORE: &str = "cli_auth_credentials_store=\"file\"";
/// Codex's built-in web search (`disabled`, `cached`, `indexed` or `live` in codex 0.162.0): `live`, since the
/// questions it is for (weather, news, prices) are about now, which a cached index may not have yet.
const WEB_SEARCH: &str = "live";
/// The user's brain setting and Anthropic API key, in the config directory (mode 0600). A file of its own, written
/// only through its own endpoints, because `PUT /v1/config` replaces the whole config and an older App would wipe a
/// key it doesn't know.
pub const SETTINGS_FILE: &str = "resident-settings.json";

pub fn enabled_by_env() -> bool {
    matches!(std::env::var(ENABLE_VARIABLE).as_deref(), Ok("1" | "true"))
}

#[derive(Debug, PartialEq)]
pub enum ResidentError {
    Disabled,
    NoCodex,
    NotLoggedIn,
    NoClaude,
    NoAnthropicKey,
    Rpc(String),
    Timeout,
    /// The turn ended without completing: interrupted or failed.
    TurnEnded(String),
}

impl std::fmt::Display for ResidentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(formatter, "the resident session is experimental and off; start vibebuddyd with {ENABLE_VARIABLE}=1"),
            Self::NoCodex => write!(formatter, "codex was not found on PATH (set {CODEX_VARIABLE} to its path)"),
            Self::NotLoggedIn => write!(formatter, "Codex is not logged in; run `codex login` first"),
            Self::NoClaude => {
                write!(formatter, "claude was not found on PATH (set {} to its path)", crate::resident_claude::CLAUDE_VARIABLE)
            }
            Self::NoAnthropicKey => write!(formatter, "no Anthropic API key is set for Claude Code"),
            Self::Rpc(message) => write!(formatter, "{message}"),
            Self::Timeout => write!(formatter, "the resident session took longer than {} s and was interrupted", TURN_TIMEOUT.as_secs()),
            Self::TurnEnded(message) => write!(formatter, "the turn ended without completing: {message}"),
        }
    }
}

impl From<RpcError> for ResidentError {
    fn from(error: RpcError) -> Self {
        Self::Rpc(error.to_string())
    }
}

/// Why the resident session can't take a voice command now: voice needs the user's Codex, installed and logged in,
/// or the user's Claude Code, installed, with their Anthropic API key.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    CodexMissing,
    CodexLoggedOut,
    ClaudeMissing,
    AnthropicKeyMissing,
}

/// The Agent the resident session runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Brain {
    Codex,
    Claude,
}

/// Which brain the user wants: `auto` is Codex when it is ready, else Claude Code when an Anthropic API key is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainChoice {
    #[default]
    Auto,
    Codex,
    Claude,
}

/// The last readiness check: none yet, or when it was made, what it found, and the brain that answers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Readiness {
    pub checked: Option<tokio::time::Instant>,
    pub problem: Option<Problem>,
    /// The brain voice commands go to; None while not ready under `auto`.
    pub brain: Option<Brain>,
}

/// What the user set: the brain, and their Anthropic API key (never sent back to a client).
#[derive(Clone, Default, Deserialize, Serialize)]
struct Settings {
    #[serde(default)]
    brain: BrainChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anthropic_api_key: Option<String>,
}

/// Readiness under a brain setting, from the last Codex check and whether Claude Code is there with a key.
fn decide(choice: BrainChoice, codex: Readiness, claude_installed: bool, key_set: bool) -> Readiness {
    let claude_problem = if !claude_installed {
        Some(Problem::ClaudeMissing)
    } else if !key_set {
        Some(Problem::AnthropicKeyMissing)
    } else {
        None
    };
    let codex_side = Readiness { brain: Some(Brain::Codex), ..codex };
    let claude_side = Readiness { checked: Some(tokio::time::Instant::now()), problem: claude_problem, brain: Some(Brain::Claude) };
    match choice {
        BrainChoice::Codex => codex_side,
        BrainChoice::Claude => claude_side,
        BrainChoice::Auto if codex.ready() => codex_side,
        BrainChoice::Auto if claude_problem.is_none() => claude_side,
        // Codex hasn't been looked at yet: no answer either way.
        BrainChoice::Auto if codex.checked.is_none() => Readiness::default(),
        // A key says the user means Claude Code, so its problem is the one to fix; else Codex's.
        BrainChoice::Auto => Readiness {
            checked: codex.checked,
            problem: if key_set { claude_problem } else { codex.problem },
            brain: None,
        },
    }
}

impl Readiness {
    pub fn ready(&self) -> bool {
        self.checked.is_some() && self.problem.is_none()
    }
}

/// What the resident session said in answer to one voice command.
#[derive(Debug, Serialize)]
pub struct Answer {
    pub thread_id: String,
    pub replies: Vec<String>,
}

/// One past voice command and the spoken replies to it, read back from the thread.
#[derive(Debug, PartialEq, Serialize)]
pub struct Exchange {
    pub said: String,
    pub replies: Vec<String>,
}

/// How the app-server is reached: the real `codex` child, or an in-memory fake in tests.
pub enum Launcher {
    Codex,
    #[cfg(test)]
    Fake(Box<dyn Fn() -> Pipes + Send + Sync>),
}

pub struct Pipes {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    /// Killed when the session is dropped.
    pub child: Option<tokio::process::Child>,
}

struct Session {
    client: Client,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    thread_id: String,
    /// Who its instructions make the buddy: they are set only when the thread starts or is resumed.
    persona: Persona,
    _child: Option<tokio::process::Child>,
}

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread_id: Option<String>,
    /// The Codex version the thread was last resumed with, for diagnosing a protocol change.
    codex_version: Option<String>,
    /// The Character whose persona the thread last answered in.
    #[serde(default)]
    persona: Option<String>,
}

pub struct Resident {
    enabled: bool,
    /// The working directory the resident thread runs in; the daemon owns it.
    dir: PathBuf,
    /// The same directory with symlinks resolved, as Codex may report it in hooks.
    canonical_dir: PathBuf,
    state_file: PathBuf,
    /// The Codex home the app-server runs in; the daemon owns it.
    codex_home: PathBuf,
    /// The user's own Codex home, whose login and memories the resident session shares.
    user_codex_home: PathBuf,
    launcher: Launcher,
    activities: Arc<Mutex<ActivityTracker>>,
    /// Held for a whole turn: one voice command at a time, the rest wait here in order.
    session: Mutex<Option<Session>>,
    /// Read synchronously by the hook filter.
    thread_id: std::sync::Mutex<Option<String>>,
    /// The Character whose persona the thread last answered in (kept in `resident.json`).
    spoke_as: std::sync::Mutex<Option<String>>,
    /// The persona of the last voice command, for starting the session when no command asks (history).
    persona: std::sync::Mutex<Option<Persona>>,
    /// Bumped when a turn ends or the thread changes, so a client can tell the history moved without reading it.
    revision: std::sync::atomic::AtomicU64,
    /// The last Codex check (its `brain` unused).
    codex_readiness: std::sync::Mutex<Readiness>,
    /// Readiness under the brain setting; watchers hear when `ready()` flips.
    readiness: watch::Sender<Readiness>,
    /// The session on the user's Claude Code.
    claude: ClaudeBrain,
    settings: std::sync::Mutex<Settings>,
    /// Where the settings are kept; None keeps them in memory only (tests).
    settings_file: Option<PathBuf>,
    /// What the readiness check finds with the fake app-server.
    #[cfg(test)]
    fake_problem: std::sync::Mutex<Option<Problem>>,
}

impl Resident {
    /// `state_dir` is the daemon's state directory: the thread id is kept in `resident.json` there, the session
    /// works in `resident/`, and its Codex home is `resident-codex/`.
    pub fn new(enabled: bool, state_dir: &Path, launcher: Launcher, activities: Arc<Mutex<ActivityTracker>>) -> Self {
        let dir = state_dir.join("resident");
        let state_file = state_dir.join("resident.json");
        let saved = load(&state_file);
        let canonical_dir = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        Self {
            enabled,
            dir,
            canonical_dir,
            state_file,
            codex_home: state_dir.join(CODEX_HOME_DIR),
            user_codex_home: user_codex_home(),
            launcher,
            claude: ClaudeBrain::new(state_dir, crate::resident_claude::Launcher::Claude, activities.clone()),
            activities,
            session: Mutex::new(None),
            thread_id: std::sync::Mutex::new(saved.thread_id),
            spoke_as: std::sync::Mutex::new(saved.persona),
            persona: std::sync::Mutex::new(None),
            revision: std::sync::atomic::AtomicU64::new(0),
            codex_readiness: std::sync::Mutex::new(Readiness::default()),
            readiness: watch::channel(Readiness::default()).0,
            settings: std::sync::Mutex::new(Settings::default()),
            settings_file: None,
            #[cfg(test)]
            fake_problem: std::sync::Mutex::new(None),
        }
    }

    /// Keeps the brain setting and the Anthropic API key in `path` (see [`SETTINGS_FILE`]), reading them now.
    pub fn with_settings_file(mut self, path: PathBuf) -> Self {
        let settings = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok());
        self.settings = std::sync::Mutex::new(settings.unwrap_or_default());
        self.settings_file = Some(path);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_claude(mut self, claude: ClaudeBrain) -> Self {
        self.claude = claude;
        self
    }

    pub fn brain_setting(&self) -> BrainChoice {
        self.settings.lock().expect("settings lock").brain
    }

    pub fn anthropic_key_set(&self) -> bool {
        self.anthropic_key().is_some()
    }

    fn anthropic_key(&self) -> Option<String> {
        self.settings.lock().expect("settings lock").anthropic_api_key.clone()
    }

    pub async fn set_brain(&self, choice: BrainChoice) -> std::io::Result<()> {
        self.change_settings(|settings| settings.brain = choice)?;
        info!(?choice, "the resident session's brain setting changed");
        self.check_readiness_now().await;
        Ok(())
    }

    /// Sets or clears the user's Anthropic API key. A running Claude Code session is stopped, so the next command
    /// starts one with the new key.
    pub async fn set_anthropic_key(&self, key: Option<String>) -> std::io::Result<()> {
        let key = key.map(|key| key.trim().to_owned()).filter(|key| !key.is_empty());
        let set = key.is_some();
        self.change_settings(|settings| settings.anthropic_api_key = key)?;
        info!(set, "the Anthropic API key for the resident session changed");
        self.claude.stop().await;
        self.check_readiness_now().await;
        Ok(())
    }

    fn change_settings(&self, change: impl FnOnce(&mut Settings)) -> std::io::Result<()> {
        let mut settings = self.settings.lock().expect("settings lock");
        let mut changed = settings.clone();
        change(&mut changed);
        if let Some(path) = &self.settings_file {
            let text = serde_json::to_string_pretty(&changed).map_err(std::io::Error::other)?;
            crate::resident_claude::write_private(path, &text)?;
        }
        *settings = changed;
        Ok(())
    }

    /// The brain a voice command goes to now: the ready one, else the one the setting points at (Codex under
    /// `auto`), whose start then says what is missing.
    pub fn brain(&self) -> Brain {
        self.readiness().brain.unwrap_or(match self.brain_setting() {
            BrainChoice::Claude => Brain::Claude,
            BrainChoice::Codex | BrainChoice::Auto => Brain::Codex,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The session voice commands go to now: the Codex thread or the Claude Code session.
    pub fn thread_id(&self) -> Option<String> {
        match self.brain() {
            Brain::Codex => self.codex_thread_id(),
            Brain::Claude => self.claude.session_id(),
        }
    }

    fn codex_thread_id(&self) -> Option<String> {
        self.thread_id.lock().expect("thread id lock").clone()
    }

    /// A change marker for the history, kept in memory only: it starts at 0 with the daemon and goes up each time a
    /// voice command's turn ends (answered or not) and when the thread changes.
    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn bump_revision(&self) {
        self.revision.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// The last readiness check, as it stands; see [`Resident::check_readiness`].
    pub fn readiness(&self) -> Readiness {
        *self.readiness.borrow()
    }

    /// Whether the session can take a voice command: on, and the last check found the brain the setting picks
    /// ready (Codex installed and logged in, or Claude Code installed with a key).
    pub fn ready(&self) -> bool {
        self.enabled && self.readiness().ready()
    }

    /// Changes each time [`Resident::ready`] flips.
    pub fn watch_ready(&self) -> watch::Receiver<Readiness> {
        self.readiness.subscribe()
    }

    /// Checks again whether `codex` is there and the user logged in, unless the last check is younger than
    /// [`READINESS_TTL`] or the setting is Claude Code; and whether `claude` is there and a key set, which costs
    /// nothing. Starts no app-server, process or turn: it looks `codex` up as a start would and asks `codex login
    /// status` in the resident Codex home, which reads the login the app-server's `account/read` would.
    pub async fn check_readiness(&self) -> Readiness {
        if !self.enabled {
            return self.readiness();
        }
        let last = *self.codex_readiness.lock().expect("readiness lock");
        if self.brain_setting() != BrainChoice::Claude
            && !last.checked.is_some_and(|checked| checked.elapsed() < READINESS_TTL)
        {
            let problem = match &self.launcher {
                Launcher::Codex => self.login_problem().await,
                #[cfg(test)]
                Launcher::Fake(_) => *self.fake_problem.lock().unwrap(),
            };
            self.note(problem);
        } else {
            self.publish();
        }
        self.readiness()
    }

    /// The same check, but Codex is asked again whatever the age of its last answer: a setting changed.
    async fn check_readiness_now(&self) {
        self.codex_readiness.lock().expect("readiness lock").checked = None;
        self.check_readiness().await;
    }

    async fn login_problem(&self) -> Option<Problem> {
        let Some(program) = find_codex() else { return Some(Problem::CodexMissing) };
        if let Err(error) = prepare_codex_home(&self.codex_home, &self.user_codex_home) {
            warn!(%error, "cannot prepare the resident Codex home to check the login");
            return Some(Problem::CodexLoggedOut);
        }
        let status = codex_command(&program, &self.codex_home)
            .args(["-c", AUTH_STORE, "login", "status"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        match status {
            Ok(status) if status.success() => None,
            Ok(_) => Some(Problem::CodexLoggedOut),
            Err(error) => {
                warn!(program = %program.display(), %error, "cannot run codex to check the login");
                Some(Problem::CodexMissing)
            }
        }
    }

    /// Records what a Codex check or start found; see [`Resident::publish`].
    pub(crate) fn note(&self, problem: Option<Problem>) {
        *self.codex_readiness.lock().expect("readiness lock") =
            Readiness { checked: Some(tokio::time::Instant::now()), problem, brain: None };
        self.publish();
    }

    /// Works out readiness under the brain setting; watchers are told only when it flips. A change of brain moves
    /// the revision, since the history shown is the other brain's now.
    fn publish(&self) {
        let codex = *self.codex_readiness.lock().expect("readiness lock");
        let now = decide(self.brain_setting(), codex, self.claude.installed(), self.anthropic_key_set());
        let mut switched = false;
        self.readiness.send_if_modified(|readiness| {
            let flipped = readiness.ready() != now.ready();
            if (readiness.problem, readiness.brain) != (now.problem, now.brain) {
                info!(problem = ?now.problem, brain = ?now.brain, "resident session readiness changed");
            }
            switched = readiness.brain.is_some() && now.brain.is_some() && readiness.brain != now.brain;
            *readiness = now;
            flipped
        });
        if switched {
            self.bump_revision();
        }
    }

    /// Whether a Codex hook event comes from the resident session: by its thread, or by its working directory. The
    /// user's own hooks don't load in the resident Codex home, but system and managed ones may.
    pub fn owns_hook(&self, session_id: &str, thread_id: Option<&str>, cwd: Option<&str>) -> bool {
        if let Some(resident) = self.thread_id.lock().expect("thread id lock").as_deref()
            && (session_id == resident || thread_id == Some(resident))
        {
            return true;
        }
        self.owns_cwd(cwd)
    }

    /// Whether a Claude Code hook event comes from the resident session: by its session, or by its working
    /// directory. Its config dir loads none of the user's hooks, but managed ones may.
    pub fn owns_claude_hook(&self, session_id: &str, cwd: Option<&str>) -> bool {
        self.claude.owns_session(session_id) || self.owns_cwd(cwd)
    }

    fn owns_cwd(&self, cwd: Option<&str>) -> bool {
        cwd.map(Path::new)
            .is_some_and(|cwd| cwd.starts_with(&self.dir) || cwd.starts_with(&self.canonical_dir))
    }

    /// Hands one voice command to the resident session and waits for its spoken replies.
    pub async fn say(&self, text: &str, wearing: &Wearing) -> Result<Answer, ResidentError> {
        let mut replies = Vec::new();
        let thread_id = self.say_streaming(text, wearing, &mut |reply| replies.push(reply.to_owned())).await?;
        Ok(Answer { thread_id, replies })
    }

    /// Hands one voice command to the resident session, answered as the Character the buddy is `wearing`; each
    /// spoken reply goes to `on_reply` the moment the session calls `speak`, not when the turn completes, which in
    /// testing was 4 to 6 seconds later. Returns the thread id once the turn has completed.
    pub async fn say_streaming(
        &self,
        text: &str,
        wearing: &Wearing,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        if !self.enabled {
            return Err(ResidentError::Disabled);
        }
        let persona = Persona::of(wearing);
        *self.persona.lock().expect("persona lock") = Some(persona.clone());
        let result = match self.brain() {
            Brain::Codex => self.say_codex(text, &persona, on_reply).await,
            Brain::Claude => self.say_claude(text, &persona, on_reply).await,
        };
        // Even a failed or timed-out turn may have left the command in the thread.
        self.bump_revision();
        result
    }

    async fn say_codex(
        &self,
        text: &str,
        persona: &Persona,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        let mut slot = self.session.lock().await;
        // A running thread keeps the instructions it was resumed with, so a new Character resumes it again.
        if slot.as_ref().is_some_and(|session| session.persona != *persona) {
            info!(character = persona.id, "the Character changed; resuming the resident thread with its persona");
            *slot = None;
        }
        let session = self.running(&mut slot, persona).await?;
        let spoke_as = self.spoke_as.lock().expect("spoke as lock").clone();
        let changed = spoke_as.as_deref() != Some(persona.id);
        let note = (changed && spoke_as.is_some()).then(|| change_note(persona));
        let result = self.run_turn(session, note.as_deref(), text, on_reply).await;
        if session.client.is_closed() {
            *slot = None;
        }
        if result.is_ok() && changed {
            self.remember_persona(persona.id);
        }
        result
    }

    /// Keeps the Character the thread now answers as, so a change is noticed after a restart too.
    fn remember_persona(&self, id: &str) {
        *self.spoke_as.lock().expect("spoke as lock") = Some(id.to_owned());
        let mut saved = load(&self.state_file);
        saved.persona = Some(id.to_owned());
        if let Err(error) = save(&self.state_file, &saved) {
            warn!(%error, "failed to save the resident session's Character");
        }
    }

    async fn say_claude(
        &self,
        text: &str,
        persona: &Persona,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        let key = self.anthropic_key().ok_or(ResidentError::NoAnthropicKey)?;
        let instructions = instructions(persona, None, Brain::Claude);
        let result = self.claude.say_streaming(&key, &instructions, persona, text, on_reply).await;
        if result == Err(ResidentError::NoClaude) {
            self.publish();
        }
        result
    }

    /// The persona of the last voice command, else the one the thread last answered in.
    fn last_persona(&self) -> Persona {
        if let Some(persona) = self.persona.lock().expect("persona lock").clone() {
            return persona;
        }
        let character = self.spoke_as.lock().expect("spoke as lock").clone();
        Persona::of(&Wearing { character, address: None })
    }

    /// Starts the app-server and resumes the thread ahead of the first voice command, which otherwise waits for
    /// both, as the Character the buddy is `wearing`. Does nothing when switched off or already running; a failure
    /// is logged and the next command tries again.
    pub async fn warm(&self, wearing: &Wearing) {
        if !self.enabled {
            return;
        }
        self.check_readiness().await;
        let persona = Persona::of(wearing);
        *self.persona.lock().expect("persona lock") = Some(persona.clone());
        if self.brain() == Brain::Claude {
            if let Some(key) = self.anthropic_key() {
                self.claude.warm(&key, &instructions(&persona, None, Brain::Claude)).await;
            }
            return;
        }
        let mut slot = self.session.lock().await;
        match self.running(&mut slot, &persona).await {
            Ok(session) => info!(thread = %session.thread_id, "the resident session is ready"),
            Err(error) => warn!(%error, "the resident session could not be started ahead of time"),
        }
    }

    /// The last `limit` voice commands and their spoken replies, oldest first, read from the thread through the
    /// app-server (the daemon keeps no record of its own). Empty while there is no thread yet.
    pub async fn history(&self, limit: usize) -> Result<Vec<Exchange>, ResidentError> {
        if !self.enabled {
            return Err(ResidentError::Disabled);
        }
        if self.brain() == Brain::Claude {
            return Ok(self.claude.history(limit));
        }
        if self.codex_thread_id().is_none() {
            return Ok(Vec::new());
        }
        let mut slot = self.session.lock().await;
        let session = self.running(&mut slot, &self.last_persona()).await?;
        let params = json!({"threadId": session.thread_id, "limit": limit, "sortDirection": "desc", "itemsView": "full"});
        let listed = request(&session.client, "thread/turns/list", params).await;
        if session.client.is_closed() {
            *slot = None;
        }
        let turns = listed?.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(turns.iter().rev().filter_map(exchange_of).collect())
    }

    /// The running session, started (again) if there is none or its app-server has exited.
    async fn running<'a>(&self, slot: &'a mut Option<Session>, persona: &Persona) -> Result<&'a mut Session, ResidentError> {
        if slot.as_ref().is_some_and(|session| session.client.is_closed()) {
            warn!("the resident app-server exited; starting it again");
            *slot = None;
        }
        if slot.is_none() {
            let started = self.start(persona).await;
            // A start is a readiness check too, and the most recent one.
            match &started {
                Ok(_) => self.note(None),
                Err(ResidentError::NoCodex) => self.note(Some(Problem::CodexMissing)),
                Err(ResidentError::NotLoggedIn) => self.note(Some(Problem::CodexLoggedOut)),
                Err(_) => {}
            }
            *slot = Some(started?);
        }
        Ok(slot.as_mut().expect("session was just started"))
    }

    async fn start(&self, persona: &Persona) -> Result<Session, ResidentError> {
        prepare_codex_home(&self.codex_home, &self.user_codex_home)
            .map_err(|error| ResidentError::Rpc(format!("cannot prepare {}: {error}", self.codex_home.display())))?;
        let (pipes, version) = match &self.launcher {
            Launcher::Codex => spawn_codex(&self.codex_home).await?,
            #[cfg(test)]
            Launcher::Fake(connect) => (connect(), None),
        };
        let (client, incoming) = Client::spawn(pipes.reader, pipes.writer);
        request(
            &client,
            "initialize",
            json!({
                "clientInfo": {"name": "vibebuddy", "title": "VibeBuddy", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true},
            }),
        )
        .await?;
        client.notify("initialized").await?;
        // Only the user's existing login is used; a login flow is never started from here.
        let account = request(&client, "account/read", json!({})).await?;
        if account.get("account").is_none_or(Value::is_null)
            && account.get("requiresOpenaiAuth").and_then(Value::as_bool) != Some(false)
        {
            if !self.user_codex_home.join("auth.json").exists() {
                warn!(
                    codex_home = %self.user_codex_home.display(),
                    "no auth.json in the user's Codex home: a login kept in the keychain can't be shared with the resident session"
                );
            }
            return Err(ResidentError::NotLoggedIn);
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| ResidentError::Rpc(format!("cannot create {}: {error}", self.dir.display())))?;
        let thread_id = match self.codex_thread_id() {
            // There is one resident thread for good. Only a thread that no longer exists is replaced (a thread
            // saved before the resident session had its own Codex home is one); one that can't be resumed for
            // another reason (another Codex process is its writer) fails this command and is tried again with the
            // next.
            Some(saved) => match self.resume(&client, &saved, persona).await {
                Err(ResidentError::Rpc(message)) if !client.is_closed() && thread_is_gone(&message) => {
                    let thread = self.start_thread(&client, persona).await?;
                    warn!(old_thread = %saved, new_thread = %thread, %message, "the resident thread no longer exists; started a new one");
                    thread
                }
                Err(ResidentError::Rpc(message)) if message.contains("active writer") => {
                    warn!(thread = %saved, "another Codex process is the resident thread's writer");
                    return Err(ResidentError::Rpc(message));
                }
                other => other?,
            },
            None => self.start_thread(&client, persona).await?,
        };
        let previous = self.thread_id.lock().expect("thread id lock").replace(thread_id.clone());
        if previous.as_deref() != Some(thread_id.as_str()) {
            self.bump_revision();
        }
        let spoke_as = self.spoke_as.lock().expect("spoke as lock").clone();
        let saved = Saved { thread_id: Some(thread_id.clone()), codex_version: version, persona: spoke_as };
        if let Err(error) = save(&self.state_file, &saved) {
            warn!(%error, "failed to save the resident thread id");
        }
        info!(thread = %thread_id, "resident session ready");
        Ok(Session { client, incoming, thread_id, persona: persona.clone(), _child: pipes.child })
    }

    async fn start_thread(&self, client: &Client, persona: &Persona) -> Result<String, ResidentError> {
        let mut params = self.thread_params(persona);
        params["dynamicTools"] = tools();
        let started = request(client, "thread/start", params).await?;
        thread_id_of(&started)
    }

    async fn resume(&self, client: &Client, thread_id: &str, persona: &Persona) -> Result<String, ResidentError> {
        // Dynamic tools live with the thread and survive a resume; the instructions are sent again so a change
        // (a new Character) takes effect.
        let mut params = self.thread_params(persona);
        params["threadId"] = json!(thread_id);
        let resumed = request(client, "thread/resume", params).await?;
        thread_id_of(&resumed)
    }

    fn thread_params(&self, persona: &Persona) -> Value {
        json!({
            "cwd": self.dir,
            "sandbox": "read-only",
            "approvalPolicy": "on-request",
            "developerInstructions": instructions(persona, memory_summary(&self.user_codex_home).as_deref(), Brain::Codex),
        })
    }

    async fn run_turn(
        &self,
        session: &mut Session,
        note: Option<&str>,
        text: &str,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        // Whatever an earlier, timed-out turn left behind still gets its answer, but no longer counts.
        while let Ok(message) = session.incoming.try_recv() {
            self.handle(&session.client, message, &mut |_| {}).await;
        }
        let input: Vec<Value> =
            note.into_iter().chain([text]).map(|text| json!({"type": "text", "text": text})).collect();
        let started =
            request(&session.client, "turn/start", json!({"threadId": session.thread_id, "input": input})).await?;
        let turn_id = started.pointer("/turn/id").and_then(Value::as_str).unwrap_or_default().to_owned();
        let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
        loop {
            let message = match tokio::time::timeout_at(deadline, session.incoming.recv()).await {
                Err(_) => {
                    let _ = session
                        .client
                        .request("turn/interrupt", json!({"threadId": session.thread_id, "turnId": turn_id}))
                        .await;
                    return Err(ResidentError::Timeout);
                }
                Ok(None) => return Err(RpcError::Closed.into()),
                Ok(Some(message)) => message,
            };
            if let Incoming::Notification { method, params } = &message
                && method == "turn/completed"
                && params.pointer("/turn/id").and_then(Value::as_str) == Some(turn_id.as_str())
            {
                return match params.pointer("/turn/status").and_then(Value::as_str) {
                    Some("completed") => Ok(session.thread_id.clone()),
                    status => Err(ResidentError::TurnEnded(
                        params
                            .pointer("/turn/error/message")
                            .and_then(Value::as_str)
                            .or(status)
                            .unwrap_or("unknown")
                            .to_owned(),
                    )),
                };
            }
            self.handle(&session.client, message, on_reply).await;
        }
    }

    /// Answers a server-to-client request; notifications other than the turn's end need nothing.
    async fn handle(&self, client: &Client, message: Incoming, on_reply: &mut (dyn FnMut(&str) + Send)) {
        let Incoming::Request { id, method, params } = message else {
            return;
        };
        let answered = match decline(&method) {
            Some(result) => {
                info!(%method, "declined a request from the resident session");
                client.respond(id, result).await
            }
            None if method == "item/tool/call" => {
                let output = self.call_tool(&params, on_reply).await;
                client.respond(id, output).await
            }
            None if method == "currentTime/read" => {
                client.respond(id, json!({"currentTimeAt": chrono::Utc::now().timestamp()})).await
            }
            None => {
                info!(%method, "refused an unsupported request from the resident session");
                client.respond_error(id, -32601, "not supported by VibeBuddy").await
            }
        };
        if let Err(error) = answered {
            warn!(%error, "failed to answer the resident session");
        }
    }

    async fn call_tool(&self, params: &Value, on_reply: &mut (dyn FnMut(&str) + Send)) -> Value {
        let tool = params.get("tool").and_then(Value::as_str).unwrap_or_default();
        let memories = self.user_codex_home.join("memories");
        let (success, text) = run_tool(tool, &params["arguments"], &self.activities, Some(&memories), on_reply).await;
        json!({"success": success, "contentItems": [{"type": "inputText", "text": text}]})
    }
}

/// Runs one of our tools, for either brain: whether it succeeded, and what the session reads back. `memories` is
/// the user's Codex memories folder; without it there is no `search_memories`.
pub(crate) async fn run_tool(
    tool: &str,
    arguments: &Value,
    activities: &Mutex<ActivityTracker>,
    memories: Option<&Path>,
    on_reply: &mut (dyn FnMut(&str) + Send),
) -> (bool, String) {
    match (tool, memories) {
        ("speak", _) => {
            let text = arguments.get("text").and_then(Value::as_str).unwrap_or_default().trim();
            if text.is_empty() {
                (false, "text is empty".to_owned())
            } else {
                info!(chars = text.chars().count(), "the resident session spoke");
                on_reply(text);
                (true, "spoken".to_owned())
            }
        }
        ("list_activities", _) => {
            let report = activities.lock().await.report();
            (true, json!(report).to_string())
        }
        ("search_memories", Some(memories)) => {
            let query = arguments.get("query").and_then(Value::as_str).unwrap_or_default();
            (true, search_memories(memories, query))
        }
        (other, _) => (false, format!("unknown tool {other}")),
    }
}

/// The answer that declines a server-to-client request asking for permission or input; None if it isn't one.
fn decline(method: &str) -> Option<Value> {
    Some(match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({"decision": "decline"}),
        "execCommandApproval" | "applyPatchApproval" => json!({"decision": "denied"}),
        "item/permissions/requestApproval" => json!({"permissions": {}}),
        "mcpServer/elicitation/request" => json!({"action": "decline"}),
        "item/tool/requestUserInput" => json!({"answers": {}}),
        _ => return None,
    })
}

/// The tools the resident session gets. Nothing here may run a command or write a file.
pub(crate) fn tools() -> Value {
    json!([
        {
            "type": "function",
            "name": "speak",
            "description": "Say something to the user out loud through the box. This is the only way the user hears you.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "description": "One or two short spoken sentences."}},
                "required": ["text"],
                "additionalProperties": false,
            },
        },
        {
            "type": "function",
            "name": "list_activities",
            "description": "What the user's coding Agents are doing and have done. `activities`: what runs right now, \
                newest first: agent, task card title, project, state (working or needs_input), where it runs, and \
                seconds in that state. `recently_ended`: the last tasks that ended since VibeBuddy started, newest \
                first: agent, title, project, how it ended (done, failed, stopped: interrupted or closed with no \
                outcome, or expired: nothing heard for too long) and `ended_s`, seconds ago. `today`: tasks done, \
                times an Agent asked for input, and seconds with at least one Agent busy, today. Use it for what is \
                happening now, what just finished, and how today went.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        },
        {
            "type": "function",
            "name": "search_memories",
            "description": "Search the user's Codex memories (notes Codex keeps from the user's past sessions) for lines \
                containing any of the words in `query`, case-insensitive. Returns the matching lines, each with the \
                line before and after, by file.",
            "inputSchema": {
                "type": "object",
                "properties": {"query": {"type": "string", "description": "A few keywords."}},
                "required": ["query"],
                "additionalProperties": false,
            },
        },
    ])
}

/// The developer instructions: the buddy as the Character it wears (CONTEXT.md, Persona), in that Character's one
/// language and with the user's form of address. `memories` is the user's Codex memory summary, if any. `brain`
/// says where today's date is: in Codex's environment context, or in the context block the daemon puts before each
/// voice command for Claude Code.
pub(crate) fn instructions(persona: &Persona, memories: Option<&str>, brain: Brain) -> String {
    let Persona { name, text, language, address, .. } = persona;
    let address = address.map_or_else(String::new, |words| {
        format!(
            " The user likes to be called \"{words}\": call them that now and then, naturally, as your lines do, not \
             in every reply."
        )
    });
    let memories = memories.map_or_else(String::new, |summary| {
        format!(
            "\n\nWhat Codex remembers about the user from their past sessions, in summary. For more, call \
             `search_memories`. Use it when it helps; never read it out.\n<memory_summary>\n{summary}\n</memory_summary>"
        )
    });
    let date = match brain {
        Brain::Codex => "today's date and the user's time zone are in the environment context",
        Brain::Claude => "the user's local date, time and time zone come with each voice command, in a context block",
    };
    format!(
        "You are the buddy of VibeBuddy: a small desk box that watches the user's coding Agents (Codex, Claude Code, \
         OpenCode, GitHub Copilot CLI, Pi, GitHub Actions) and speaks up for them. Right now you are {name}, the \
         Character the user picked. This is {name}'s persona, written for the short lines {name} says when an Agent \
         needs the user or is done; speak every reply as {name}, in this persona:\n<persona>\n{text}\n</persona>\n\
         You speak {language} only. The user may talk to you in any language and you understand it, but you always \
         reply in {language}: a question asked in another language still gets its answer in {language}, never in the \
         language it was asked in, whatever earlier replies or the memories below say about language.{address}\n\
         Every user message is one voice command, transcribed from speech. Answer only by calling the `speak` tool, \
         with one or two short sentences meant to be heard, not read: no Markdown, lists, code, paths or URLs. \
         Do not write any other answer: once you have called `speak`, end your turn without writing anything else.\n\
         To know what the Agents are doing, have just finished or did today, call `list_activities`; never guess. It \
         has the live tasks, the ones that ended lately with how and how long ago (`recently_ended`), and today's \
         counts (`today`). Titles there are short upper-case task-card names; say them naturally.\n\
         Never state a fact you have not seen in a tool result, a web search result or the memories below: not the \
         weather, news, prices, scores, schedules or anything else about now, and nothing about the user's world. \
         For things about now, search the web, and answer only from what the search found; {date}, so work out \
         today and tomorrow from them. If you can't \
         find out, say so plainly in one sentence; never make up an answer that sounds likely.\n\
         You do no work yourself: run no commands, read or edit no files. If the user asks for work to be done, say \
         briefly that you can't start it yet.{memories}"
    )
}

/// A context line put before the next voice command once the Character has changed, so the session takes up the
/// new voice rather than carrying on in the old one's. The history leaves it out, as it does every context block.
pub(crate) fn change_note(persona: &Persona) -> String {
    let Persona { name, language, .. } = persona;
    format!(
        "{CONTEXT_PREFIX}The user just switched you to another Character: from now on you are {name}. Speak only as \
         {name}, in {language}, whatever your earlier replies in this conversation sounded like.</context>"
    )
}

fn thread_id_of(response: &Value) -> Result<String, ResidentError> {
    response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ResidentError::Rpc("app-server returned no thread id".to_owned()))
}

async fn request(client: &Client, method: &str, params: Value) -> Result<Value, ResidentError> {
    match tokio::time::timeout(REQUEST_TIMEOUT, client.request(method, params)).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(ResidentError::Rpc(format!("app-server did not answer {method}"))),
    }
}

/// Starts `codex app-server` in the resident session's Codex home and checks its version against the one this was
/// tested with.
async fn spawn_codex(codex_home: &Path) -> Result<(Pipes, Option<String>), ResidentError> {
    let program = find_codex().ok_or(ResidentError::NoCodex)?;
    let version = codex_command(&program, codex_home)
        .arg("--version")
        .output()
        .await
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.split_whitespace().last().map(str::to_owned));
    match version.as_deref() {
        Some(TESTED_CODEX_VERSION) => {}
        other => warn!(
            version = other.unwrap_or("unknown"),
            tested = TESTED_CODEX_VERSION,
            "the resident session was tested with another Codex version; app-server is experimental and may differ"
        ),
    }
    let base = tool_overrides();
    let servers = enabled_mcp_servers(&program, codex_home, &base).await;
    let mut child = codex_command(&program, codex_home)
        .args(app_server_args(&base, &servers))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| ResidentError::Rpc(format!("cannot start {}: {error}", program.display())))?;
    info!(program = %program.display(), mcp_servers_off = servers.len(), "started codex app-server for the resident session");
    let reader = child.stdout.take().expect("stdout is piped");
    let writer = child.stdin.take().expect("stdin is piped");
    Ok((Pipes { reader: Box::new(reader), writer: Box::new(writer), child: Some(child) }, version))
}

/// `codex` run in the resident session's own Codex home. The user's `config.toml`, `AGENTS.md`, hooks, plugins and
/// MCP servers live in theirs, so none of them load; Codex's system and managed config still do.
fn codex_command(program: &Path, codex_home: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.env("CODEX_HOME", codex_home);
    command
}

/// Features that give a Codex thread tools of its own: the resident session gets none of them, only ours. Most are
/// on by default even in an empty Codex home. Plugins and apps (connectors) bring tools and MCP servers; the rest
/// are built-in tools. `memories` (off by default) would also start Codex's memory pipeline, which rewrites the
/// memories folder of the Codex home it runs in; the daemon gives the session the user's memories itself.
/// `shell_snapshot` runs the user's login shell at every start and saves its whole environment, API keys included,
/// in the Codex home, for a shell tool the session doesn't have. Set as `-c features.<name>=false` rather than
/// `--disable`, which refuses a name another Codex version doesn't know.
///
/// `code_mode_host` stays on: the model calls every tool, ours included, through its `exec` JavaScript, and with
/// the host off no tool call gets through. That JavaScript has no file system, network or process (checked on
/// 2026-10-14: no `fetch`, `require` or `process`, and `import` is refused); it reaches only the thread's tools.
const FEATURES_OFF: [&str; 16] = [
    "plugins",
    "apps",
    "shell_tool",
    "unified_exec",
    "multi_agent",
    "goals",
    "memories",
    "image_generation",
    "view_image",
    "browser_use",
    "computer_use",
    "in_app_browser",
    "sleep_tool",
    "tool_suggest",
    "skill_search",
    "shell_snapshot",
];

/// The config overrides for the resident session's Codex, before MCP servers. Besides the tools:
/// - `cli_auth_credentials_store="file"`: the login is the user's `auth.json`, linked in (see [`link_auth`]). The
///   default already, but a keyring store would never find it (keyring entries are keyed by the Codex home's
///   path) and, on saving, would delete the link.
/// - `memories.generate_memories=false`: the resident thread is never summarised into memories, should the feature
///   come on.
/// - `skills.include_instructions=false` and `skills.bundled.enabled=false`: no skills. Codex lists the user's
///   own skills (`~/.agents/skills`) whatever the Codex home, and installs its bundled ones (`imagegen` and the
///   like) into it; the session can't run them, and the list costs every turn context.
/// - `web_search`: Codex's built-in web search, so a question about now is looked up rather than made up. The
///   queries go to OpenAI, the model provider.
/// - [`MODEL`] at [`REASONING_EFFORT`]: a voice command waits on the answer. Measured on 2026-10-13, a turn's first
///   `speak` came 2.7 s in on gpt-6-luna at low, against 4.3 s on gpt-6.1-sol at high.
fn tool_overrides() -> Vec<String> {
    let mut args: Vec<String> =
        FEATURES_OFF.iter().flat_map(|feature| ["-c".to_owned(), format!("features.{feature}=false")]).collect();
    for setting in [
        format!("web_search=\"{WEB_SEARCH}\""),
        AUTH_STORE.to_owned(),
        "memories.generate_memories=false".to_owned(),
        "skills.include_instructions=false".to_owned(),
        "skills.bundled.enabled=false".to_owned(),
        format!("model=\"{MODEL}\""),
        format!("model_reasoning_effort=\"{REASONING_EFFORT}\""),
    ] {
        args.extend(["-c".to_owned(), setting]);
    }
    args
}

/// `codex app-server` with the overrides and every MCP server still configured switched off. The resident Codex home
/// configures none, but system and managed config may. An override can't empty `mcp_servers` (it merges), so each
/// server is turned off by name.
fn app_server_args(base: &[String], mcp_servers: &[String]) -> Vec<String> {
    let mut args = base.to_vec();
    for name in mcp_servers {
        args.extend(["-c".to_owned(), format!("mcp_servers.{name}.enabled=false")]);
    }
    args.push("app-server".to_owned());
    args
}

/// The MCP servers that would be on, as Codex itself resolves them (`codex mcp list --json`, under the same
/// overrides, so servers that plugins bring are already gone). A name that can't be written as a bare TOML key
/// can't be switched off by an override; it is logged.
async fn enabled_mcp_servers(program: &Path, codex_home: &Path, base: &[String]) -> Vec<String> {
    let output = codex_command(program, codex_home)
        .args(base)
        .args(["mcp", "list", "--json"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    let listed = output.ok().and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
    let Some(servers) = listed.as_ref().and_then(Value::as_array) else {
        warn!("could not list the MCP servers; the resident session may see their tools");
        return Vec::new();
    };
    servers
        .iter()
        .filter(|server| server.get("enabled").and_then(Value::as_bool) != Some(false))
        .filter_map(|server| server.get("name").and_then(Value::as_str))
        .filter(|name| {
            let bare = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !bare {
                warn!(server = %name, "an MCP server name that can't be overridden; the resident session may see its tools");
            }
            bare
        })
        .map(str::to_owned)
        .collect()
}

/// `VIBEBUDDY_CODEX`, else `codex` on PATH, else where installers put it: the daemon may run with launchd's bare PATH.
fn find_codex() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CODEX_VARIABLE) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|dir| dir.join("codex")).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(home.map(|home| home.join(".local/bin/codex")))
        .chain(["/opt/homebrew/bin/codex", "/usr/local/bin/codex"].map(PathBuf::from))
        .find(|path| path.is_file())
}

/// The user's own Codex home: `CODEX_HOME`, else `~/.codex`.
fn user_codex_home() -> PathBuf {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|home| !home.is_empty()) {
        return PathBuf::from(home);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".codex")
}

/// Makes the resident session's Codex home (mode 0700) and links the user's login into it.
fn prepare_codex_home(codex_home: &Path, user_codex_home: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(codex_home)?;
    std::fs::set_permissions(codex_home, std::fs::Permissions::from_mode(0o700))?;
    link_auth(codex_home, user_codex_home)
}

/// Makes `auth.json` in the resident Codex home a symlink to the user's, so both share one login and either one's
/// token refresh is the other's. Codex 0.162.0 reads it with a plain open, refreshes by reloading it first (taking a
/// token the other side already refreshed), and saves by truncating and rewriting it in place, all through the link.
/// Anything else found there (a copy, a link elsewhere) is replaced, on every start. Without a login in the user's
/// home the link dangles and the session reports "not logged in".
fn link_auth(codex_home: &Path, user_codex_home: &Path) -> std::io::Result<()> {
    let link = codex_home.join("auth.json");
    let target = user_codex_home.join("auth.json");
    if std::fs::read_link(&link).is_ok_and(|current| current == target) {
        return Ok(());
    }
    match std::fs::remove_file(&link) {
        Ok(()) => info!("replaced the resident Codex home's auth.json with a link to the user's"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::os::unix::fs::symlink(&target, &link)
}

/// The user's Codex memory summary (`memories/memory_summary.md`), cut at Codex's own cap; None if there is none.
/// Read again at every start, so it follows the user's Codex as it consolidates.
fn memory_summary(user_codex_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(user_codex_home.join("memories").join("memory_summary.md")).ok()?;
    let text = text.trim();
    let end = text.floor_char_boundary(MEMORY_SUMMARY_MAX_BYTES);
    (!text.is_empty()).then(|| text[..end].to_owned())
}

/// Lines of the user's Codex memories (`MEMORY.md`, `raw_memories.md` and `rollout_summaries/`, as the user's Codex
/// would search them) containing any word of `query`, case-insensitive, each with the line before and after. Only
/// reads: the daemon never writes the user's memories.
fn search_memories(root: &Path, query: &str) -> String {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return "query is empty".to_owned();
    }
    let mut files = vec![root.join("MEMORY.md"), root.join("raw_memories.md")];
    if let Ok(entries) = std::fs::read_dir(root.join("rollout_summaries")) {
        let mut summaries: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).collect();
        summaries.sort();
        files.extend(summaries);
    }
    let mut out = String::new();
    'files: for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let name = file.strip_prefix(root).unwrap_or(&file).display().to_string();
        let mut shown_until = 0;
        for (index, line) in lines.iter().enumerate() {
            let lower = line.to_lowercase();
            if index < shown_until || !words.iter().any(|word| lower.contains(word.as_str())) {
                continue;
            }
            let (from, to) = (index.saturating_sub(1).max(shown_until), (index + 2).min(lines.len()));
            let block = format!("{name}:{}\n{}\n\n", from + 1, lines[from..to].join("\n"));
            if out.len() + block.len() > MEMORY_SEARCH_MAX_BYTES {
                out.push_str("(more matches left out)");
                break 'files;
            }
            out.push_str(&block);
            shown_until = to;
        }
    }
    if out.is_empty() { "no matches".to_owned() } else { out }
}

/// A turn from `thread/turns/list` as an exchange: the user's text and what was said through `speak`.
fn exchange_of(turn: &Value) -> Option<Exchange> {
    let items = turn.get("items")?.as_array()?;
    let said = items
        .iter()
        .filter(|item| item["type"] == "userMessage")
        .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
        .filter_map(|input| input["text"].as_str().map(str::to_owned))
        .filter(|text| !text.starts_with(CONTEXT_PREFIX))
        .collect::<Vec<_>>()
        .join(" ");
    let replies = items
        .iter()
        .filter(|item| item["type"] == "dynamicToolCall" && item["tool"] == "speak")
        .filter_map(|item| item.pointer("/arguments/text").and_then(Value::as_str).map(str::to_owned))
        .collect();
    (!said.is_empty()).then_some(Exchange { said, replies })
}

fn load(path: &Path) -> Saved {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(path: &Path, saved: &Saved) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?)
}

/// The app-server's answer to resuming a thread whose rollout is gone (deleted, or a different CODEX_HOME).
fn thread_is_gone(message: &str) -> bool {
    message.contains("no rollout found")
}

#[cfg(test)]
pub(crate) mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;
    use crate::activity::{ActivityId, ActivityStatus};

    /// How the fake app-server behaves, and what it saw.
    #[derive(Default)]
    pub(crate) struct Script {
        pub(crate) logged_in: bool,
        /// thread/resume fails, as if the rollout were gone.
        lose_thread: bool,
        busy_thread: bool,
        /// Ends its output instead of completing this turn, as if the process died; only once.
        pub(crate) die_on_turn: Option<usize>,
        /// Server-to-client requests made during each turn, before it completes.
        pub(crate) turn_requests: Vec<(String, Value)>,
        /// Requests and notifications from the daemon, in order.
        pub(crate) received: Vec<Value>,
        /// The daemon's answers to the server's own requests.
        answers: Vec<Value>,
        /// How many times the server was started.
        pub(crate) starts: usize,
        /// The turn completes only once this is notified, so a test can look at what came before it.
        pub(crate) hold: Option<Arc<tokio::sync::Notify>>,
    }

    pub(crate) type Shared = Arc<std::sync::Mutex<Script>>;

    pub(crate) fn fake_server(script: Shared) -> Pipes {
        script.lock().unwrap().starts += 1;
        let (daemon_side, server_side) = tokio::io::duplex(1 << 16);
        let (daemon_read, daemon_write) = tokio::io::split(daemon_side);
        tokio::spawn(async move {
            let (server_read, mut out) = tokio::io::split(server_side);
            let mut lines = BufReader::new(server_read).lines();
            let mut turns = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                let Some(method) = message.get("method").and_then(Value::as_str).map(str::to_owned) else {
                    script.lock().unwrap().answers.push(message);
                    continue;
                };
                script.lock().unwrap().received.push(message.clone());
                let id = message["id"].clone();
                let lose_thread = script.lock().unwrap().lose_thread;
                let busy_thread = script.lock().unwrap().busy_thread;
                let result = match method.as_str() {
                    "initialized" => continue,
                    "initialize" => json!({"userAgent": "fake"}),
                    "account/read" => {
                        let logged_in = script.lock().unwrap().logged_in;
                        let account = if logged_in { json!({"type": "chatgpt"}) } else { Value::Null };
                        json!({"account": account, "requiresOpenaiAuth": true})
                    }
                    "thread/start" => json!({"thread": {"id": "new-thread"}}),
                    "thread/resume" if busy_thread => {
                        let message = "thread old-thread already has an active writer";
                        let error = json!({"id": id, "error": {"code": -32600, "message": message}});
                        out.write_all(format!("{error}\n").as_bytes()).await.unwrap();
                        continue;
                    }
                    "thread/resume" if lose_thread => {
                        let error = json!({"id": id, "error": {"code": -32600, "message": "no rollout found"}});
                        out.write_all(format!("{error}\n").as_bytes()).await.unwrap();
                        continue;
                    }
                    "thread/resume" => json!({"thread": {"id": message["params"]["threadId"]}}),
                    "thread/turns/list" => json!({"data": [
                        {"id": "turn-2", "items": [
                            {"type": "userMessage", "id": "a", "content": [{"type": "text", "text": "and now?"}]},
                            {"type": "dynamicToolCall", "id": "b", "tool": "list_activities", "arguments": {}},
                            {"type": "dynamicToolCall", "id": "c", "tool": "speak", "arguments": {"text": "Still one."}},
                        ]},
                        {"id": "turn-1", "items": [
                            {"type": "userMessage", "id": "d", "content": [{"type": "text", "text": "what's up?"}]},
                            {"type": "dynamicToolCall", "id": "e", "tool": "speak", "arguments": {"text": "One task."}},
                            {"type": "dynamicToolCall", "id": "f", "tool": "speak", "arguments": {"text": "It's fine."}},
                        ]},
                    ], "nextCursor": null}),
                    "turn/start" => {
                        turns += 1;
                        let turn_id = format!("turn-{turns}");
                        let started = json!({"id": id, "result": {"turn": {"id": turn_id, "status": "inProgress"}}});
                        out.write_all(format!("{started}\n").as_bytes()).await.unwrap();
                        let requests = script.lock().unwrap().turn_requests.clone();
                        for (number, (request, params)) in requests.iter().enumerate() {
                            let request = json!({"id": format!("srv-{turns}-{number}"), "method": request, "params": params});
                            out.write_all(format!("{request}\n").as_bytes()).await.unwrap();
                        }
                        for _ in &requests {
                            let Ok(Some(line)) = lines.next_line().await else { return };
                            script.lock().unwrap().answers.push(serde_json::from_str(&line).unwrap());
                        }
                        if script.lock().unwrap().die_on_turn.take_if(|turn| *turn == turns).is_some() {
                            return;
                        }
                        let hold = script.lock().unwrap().hold.clone();
                        if let Some(hold) = hold {
                            hold.notified().await;
                        }
                        let completed = json!({"method": "turn/completed", "params": {
                            "threadId": message["params"]["threadId"],
                            "turn": {"id": turn_id, "status": "completed", "items": []},
                        }});
                        out.write_all(format!("{completed}\n").as_bytes()).await.unwrap();
                        continue;
                    }
                    _ => {
                        let error = json!({"id": id, "error": {"code": -32601, "message": "unknown"}});
                        out.write_all(format!("{error}\n").as_bytes()).await.unwrap();
                        continue;
                    }
                };
                out.write_all(format!("{}\n", json!({"id": id, "result": result})).as_bytes()).await.unwrap();
            }
        });
        Pipes { reader: Box::new(daemon_read), writer: Box::new(daemon_write), child: None }
    }

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibebuddy-resident-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A resident session on the fake app-server, whose user's Codex home is `user-codex/` in `dir`.
    pub(crate) fn resident(dir: &Path, script: &Shared, activities: Arc<Mutex<ActivityTracker>>) -> Resident {
        let script = script.clone();
        let mut resident =
            Resident::new(true, dir, Launcher::Fake(Box::new(move || fake_server(script.clone()))), activities);
        resident.user_codex_home = dir.join("user-codex");
        resident
    }

    /// The persona with no Character pack: the robot's.
    pub(crate) fn robot() -> Persona {
        Persona::of(&Wearing::default())
    }

    pub(crate) fn wearing(character: &str) -> Wearing {
        Wearing { character: Some(character.to_owned()), address: None }
    }

    pub(crate) fn logged_in() -> Shared {
        Arc::new(std::sync::Mutex::new(Script { logged_in: true, ..Script::default() }))
    }

    fn methods(script: &Shared) -> Vec<String> {
        script.lock().unwrap().received.iter().map(|message| message["method"].as_str().unwrap().to_owned()).collect()
    }

    pub(crate) fn tool_call(tool: &str, arguments: Value) -> (String, Value) {
        let params = json!({"threadId": "t", "turnId": "u", "callId": "c", "tool": tool, "arguments": arguments});
        ("item/tool/call".to_owned(), params)
    }

    #[tokio::test]
    async fn the_first_command_shakes_hands_and_starts_one_read_only_thread_with_our_tools() {
        let dir = temp_dir("start");
        let script = logged_in();
        let resident = resident(&dir, &script, Arc::default());

        let answer = resident.say("hello", &Wearing::default()).await.unwrap();
        assert_eq!(answer.thread_id, "new-thread");
        assert_eq!(methods(&script), ["initialize", "initialized", "account/read", "thread/start", "turn/start"]);
        let received = script.lock().unwrap().received.clone();
        assert_eq!(received[0]["params"]["capabilities"]["experimentalApi"], true);
        assert_eq!(received[0]["params"]["clientInfo"]["name"], "vibebuddy");
        let start = &received[3]["params"];
        assert_eq!(start["cwd"], json!(dir.join("resident")));
        assert_eq!(start["sandbox"], "read-only");
        assert_eq!(start["approvalPolicy"], "on-request");
        let instructions = start["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("speak"));
        assert!(instructions.contains("Never state a fact you have not seen in a tool result"), "{instructions}");
        assert!(instructions.contains("not the weather, news, prices"));
        assert!(instructions.contains("search the web"));
        assert!(instructions.contains("`recently_ended`"));
        let tools: Vec<&str> =
            start["dynamicTools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(tools, ["speak", "list_activities", "search_memories"]);
        let listing = start["dynamicTools"][1]["description"].as_str().unwrap();
        assert!(listing.contains("`recently_ended`") && listing.contains("`today`"), "{listing}");
        assert_eq!(received[4]["params"], json!({"threadId": "new-thread", "input": [{"type": "text", "text": "hello"}]}));
        assert!(dir.join("resident").is_dir());
        assert_eq!(load(&dir.join("resident.json")).thread_id.as_deref(), Some("new-thread"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_later_start_resumes_the_saved_thread_and_a_lost_one_is_replaced() {
        let dir = temp_dir("resume");
        let saved = Saved { thread_id: Some("old-thread".to_owned()), codex_version: None, persona: None };
        save(&dir.join("resident.json"), &saved).unwrap();
        let script = logged_in();
        let answer = resident(&dir, &script, Arc::default()).say("hi", &Wearing::default()).await.unwrap();
        assert_eq!(answer.thread_id, "old-thread");
        assert_eq!(methods(&script), ["initialize", "initialized", "account/read", "thread/resume", "turn/start"]);
        let resume = script.lock().unwrap().received[3]["params"].clone();
        assert_eq!(resume["threadId"], "old-thread");
        assert_eq!(resume["sandbox"], "read-only");
        assert_eq!(resume["approvalPolicy"], "on-request");

        let script = logged_in();
        script.lock().unwrap().lose_thread = true;
        let answer = resident(&dir, &script, Arc::default()).say("hi", &Wearing::default()).await.unwrap();
        assert_eq!(answer.thread_id, "new-thread");
        assert_eq!(methods(&script)[3..5], ["thread/resume", "thread/start"]);
        assert_eq!(load(&dir.join("resident.json")).thread_id.as_deref(), Some("new-thread"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_thread_open_elsewhere_fails_the_command_and_is_never_replaced() {
        let dir = temp_dir("busy");
        let saved = Saved { thread_id: Some("old-thread".to_owned()), codex_version: None, persona: None };
        save(&dir.join("resident.json"), &saved).unwrap();
        let script = logged_in();
        script.lock().unwrap().busy_thread = true;
        assert!(resident(&dir, &script, Arc::default()).say("hi", &Wearing::default()).await.is_err());
        assert!(!methods(&script).contains(&"thread/start".to_owned()), "{:?}", methods(&script));
        assert_eq!(load(&dir.join("resident.json")).thread_id.as_deref(), Some("old-thread"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn without_a_login_nothing_starts_and_no_login_flow_is_begun() {
        let dir = temp_dir("login");
        let script = Arc::new(std::sync::Mutex::new(Script::default()));
        let error = resident(&dir, &script, Arc::default()).say("hi", &Wearing::default()).await.unwrap_err();
        assert_eq!(error, ResidentError::NotLoggedIn);
        assert_eq!(methods(&script), ["initialize", "initialized", "account/read"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn speak_becomes_the_reply_and_list_activities_reads_the_tracker() {
        let dir = temp_dir("tools");
        let mut tracker = ActivityTracker::default();
        let id = ActivityId { session_id: "s".to_owned(), key: "s:t".to_owned() };
        tracker.observe(&id, "CX:REVIEW", ActivityStatus::Working);
        let ended = ActivityId { session_id: "e".to_owned(), key: "e:t".to_owned() };
        tracker.note_project(&ended, "VIBE-BUDDY");
        tracker.observe(&ended, "CC:DOCS", ActivityStatus::Working);
        tracker.finish(&ended, "CC:DOCS");
        let script = logged_in();
        script.lock().unwrap().turn_requests = vec![
            tool_call("list_activities", json!({})),
            tool_call("speak", json!({"text": " One task is running. "})),
            tool_call("speak", json!({"text": ""})),
            tool_call("run_shell", json!({"command": "ls"})),
        ];
        let answer = resident(&dir, &script, Arc::new(Mutex::new(tracker))).say("what's up?", &Wearing::default()).await.unwrap();
        assert_eq!(answer.replies, ["One task is running."]);

        let answers = script.lock().unwrap().answers.clone();
        let text = |index: usize| answers[index]["result"]["contentItems"][0]["text"].as_str().unwrap().to_owned();
        assert_eq!(answers[0]["id"], "srv-1-0");
        assert_eq!(answers[0]["result"]["success"], true);
        let listed: Value = serde_json::from_str(&text(0)).unwrap();
        assert_eq!(listed["activities"][0]["title"], "CX:REVIEW");
        assert_eq!(listed["activities"][0]["state"], "working");
        assert_eq!(listed["activities"].as_array().unwrap().len(), 1);
        assert_eq!(listed["recently_ended"][0]["title"], "CC:DOCS");
        assert_eq!(listed["recently_ended"][0]["project"], "VIBE-BUDDY");
        assert_eq!(listed["recently_ended"][0]["ended"], "done");
        assert!(listed["recently_ended"][0]["ended_s"].is_u64());
        assert_eq!(listed["today"]["done"], 1);
        assert!(listed["today"]["asks"].is_u64() && listed["today"]["busy_seconds"].is_u64());
        assert_eq!(answers[1]["result"]["success"], true);
        assert_eq!(answers[2]["result"]["success"], false);
        assert_eq!(answers[3]["result"]["success"], false);
        assert_eq!(text(3), "unknown tool run_shell");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_spoken_reply_is_handed_over_when_speak_is_called_not_when_the_turn_completes() {
        let dir = temp_dir("stream");
        let script = logged_in();
        let hold = Arc::new(tokio::sync::Notify::new());
        script.lock().unwrap().hold = Some(hold.clone());
        script.lock().unwrap().turn_requests = vec![tool_call("speak", json!({"text": "On it."}))];
        let resident = Arc::new(resident(&dir, &script, Arc::default()));
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let turn = tokio::spawn({
            let resident = resident.clone();
            async move { resident.say_streaming("hi", &Wearing::default(), &mut |reply| sender.send(reply.to_owned()).unwrap()).await }
        });
        assert_eq!(receiver.recv().await.as_deref(), Some("On it."));
        assert!(!turn.is_finished(), "the turn is still running");
        hold.notify_one();
        assert_eq!(turn.await.unwrap().unwrap(), "new-thread");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn approvals_and_questions_are_declined_and_other_requests_refused() {
        let dir = temp_dir("decline");
        let script = logged_in();
        script.lock().unwrap().turn_requests = [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/permissions/requestApproval",
            "mcpServer/elicitation/request",
            "item/tool/requestUserInput",
            "execCommandApproval",
            "account/chatgptAuthTokens/refresh",
        ]
        .map(|method| (method.to_owned(), json!({})))
        .to_vec();
        resident(&dir, &script, Arc::default()).say("delete everything", &Wearing::default()).await.unwrap();
        let answers = script.lock().unwrap().answers.clone();
        assert_eq!(answers[0]["result"], json!({"decision": "decline"}));
        assert_eq!(answers[1]["result"], json!({"decision": "decline"}));
        assert_eq!(answers[2]["result"], json!({"permissions": {}}));
        assert_eq!(answers[3]["result"], json!({"action": "decline"}));
        assert_eq!(answers[4]["result"], json!({"answers": {}}));
        assert_eq!(answers[5]["result"], json!({"decision": "denied"}));
        assert_eq!(answers[6]["error"]["code"], -32601);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_dead_app_server_fails_the_turn_and_is_started_again_on_the_next_command() {
        let dir = temp_dir("restart");
        let script = logged_in();
        script.lock().unwrap().die_on_turn = Some(1);
        let resident = resident(&dir, &script, Arc::default());
        assert!(matches!(resident.say("one", &Wearing::default()).await, Err(ResidentError::Rpc(_))));
        let answer = resident.say("two", &Wearing::default()).await.unwrap();
        assert_eq!(answer.thread_id, "new-thread");
        assert_eq!(script.lock().unwrap().starts, 2);
        // The second start resumes the thread the first one made.
        assert!(methods(&script).contains(&"thread/resume".to_owned()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn concurrent_commands_wait_their_turn() {
        let dir = temp_dir("queue");
        let script = logged_in();
        let resident = Arc::new(resident(&dir, &script, Arc::default()));
        let robot = Wearing::default();
        let (first, second) = tokio::join!(resident.say("one", &robot), resident.say("two", &robot));
        assert!(first.is_ok() && second.is_ok());
        let turns: Vec<Value> = script
            .lock()
            .unwrap()
            .received
            .iter()
            .filter(|message| message["method"] == "turn/start")
            .map(|message| message["params"]["input"][0]["text"].clone())
            .collect();
        assert_eq!(turns, [json!("one"), json!("two")]);
        assert_eq!(script.lock().unwrap().starts, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn off_by_default_it_starts_nothing() {
        let dir = temp_dir("off");
        let script = logged_in();
        let launch = script.clone();
        let resident = Resident::new(false, &dir, Launcher::Fake(Box::new(move || fake_server(launch.clone()))), Arc::default());
        assert_eq!(resident.say("hi", &Wearing::default()).await.unwrap_err(), ResidentError::Disabled);
        assert_eq!(script.lock().unwrap().starts, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_codex_home_is_private_and_shares_the_users_login_through_a_link() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("home");
        std::fs::create_dir_all(dir.join("user-codex")).unwrap();
        std::fs::write(dir.join("user-codex/auth.json"), "{}").unwrap();
        resident(&dir, &logged_in(), Arc::default()).say("hi", &Wearing::default()).await.unwrap();
        let home = dir.join(CODEX_HOME_DIR);
        assert_eq!(std::fs::metadata(&home).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::read_link(home.join("auth.json")).unwrap(), dir.join("user-codex/auth.json"));
        // Nothing else is put there: no config.toml.
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copied_or_misdirected_login_is_replaced_by_the_link_on_every_start() {
        let dir = temp_dir("relink");
        let (home, user) = (dir.join("home"), dir.join("user"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("auth.json"), "{\"tokens\":\"user\"}").unwrap();
        let link = home.join("auth.json");

        prepare_codex_home(&home, &user).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), user.join("auth.json"));
        prepare_codex_home(&home, &user).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), user.join("auth.json"));

        // A diverged copy, as a write by rename would leave behind.
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, "{\"tokens\":\"stale\"}").unwrap();
        prepare_codex_home(&home, &user).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), user.join("auth.json"));
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "{\"tokens\":\"user\"}");

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere.json"), &link).unwrap();
        prepare_codex_home(&home, &user).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), user.join("auth.json"));

        // A write through the link, as Codex refreshes tokens (truncate and rewrite in place), lands in the user's.
        let mut file = std::fs::OpenOptions::new().write(true).truncate(true).open(&link).unwrap();
        std::io::Write::write_all(&mut file, b"{}").unwrap();
        assert_eq!(std::fs::read_to_string(user.join("auth.json")).unwrap(), "{}");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_users_memories_are_read_into_the_instructions_and_searched_but_never_written() {
        let dir = temp_dir("memories");
        let memories = dir.join("user-codex/memories");
        std::fs::create_dir_all(memories.join("rollout_summaries")).unwrap();
        std::fs::write(memories.join("memory_summary.md"), "The user's cat is called Biscuit.\n").unwrap();
        std::fs::write(memories.join("MEMORY.md"), "# Pets\nCat: Biscuit\nlikes boxes\n\n# Work\nRust daemon\n").unwrap();
        std::fs::write(memories.join("rollout_summaries/a.md"), "nothing here\nbiscuit again\n").unwrap();
        let before: Vec<_> = walk(&memories);
        let script = logged_in();
        script.lock().unwrap().turn_requests = vec![
            tool_call("search_memories", json!({"query": "BISCUIT"})),
            tool_call("search_memories", json!({"query": "zebra"})),
        ];
        resident(&dir, &script, Arc::default()).say("what's my cat called?", &Wearing::default()).await.unwrap();

        let start = script.lock().unwrap().received[3]["params"].clone();
        let instructions = start["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("<memory_summary>\nThe user's cat is called Biscuit.\n</memory_summary>"));
        let answers = script.lock().unwrap().answers.clone();
        let found = answers[0]["result"]["contentItems"][0]["text"].as_str().unwrap();
        assert!(found.contains("MEMORY.md:1\n# Pets\nCat: Biscuit\nlikes boxes"), "{found}");
        assert!(found.contains("rollout_summaries/a.md:1\nnothing here\nbiscuit again"), "{found}");
        assert!(!found.contains("Rust daemon"));
        assert_eq!(answers[1]["result"]["contentItems"][0]["text"], "no matches");
        assert_eq!(walk(&memories), before, "the memories folder was changed");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every path under `root` with its contents.
    fn walk(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(root).unwrap().map(Result::unwrap) {
            let path = entry.path();
            if path.is_dir() {
                found.push((path.clone(), Vec::new()));
                found.extend(walk(&path));
            } else {
                found.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
        found.sort();
        found
    }

    #[test]
    fn without_memories_the_instructions_say_nothing_of_them_and_a_long_summary_is_cut() {
        assert!(!instructions(&robot(), None, Brain::Codex).contains("memory"));
        let dir = temp_dir("summary");
        std::fs::create_dir_all(dir.join("memories")).unwrap();
        assert_eq!(memory_summary(&dir), None);
        std::fs::write(dir.join("memories/memory_summary.md"), "记".repeat(5_000)).unwrap();
        let summary = memory_summary(&dir).unwrap();
        assert!(summary.len() <= MEMORY_SUMMARY_MAX_BYTES && summary.len() > MEMORY_SUMMARY_MAX_BYTES - 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_instructions_make_the_buddy_the_character_in_its_one_language() {
        let ada = instructions(&Persona::of(&wearing("ada")), None, Brain::Codex);
        assert!(ada.contains("Right now you are Ada") && ada.contains("Londoner"), "{ada}");
        assert!(ada.contains("always reply in English"));
        assert!(!ada.contains("in the language the user spoke"));
        let xiaohe = instructions(&Persona::of(&wearing("wanwanxiaohe")), None, Brain::Claude);
        assert!(xiaohe.contains("Right now you are Xiaohe (小何)") && xiaohe.contains("Taiwan"), "{xiaohe}");
        assert!(xiaohe.contains("always reply in Chinese"));
        let robot = instructions(&robot(), None, Brain::Codex);
        assert!(robot.contains("Right now you are VibeBuddy") && robot.contains("always reply in English"));
        // Every rule is still there, whoever the buddy is.
        for text in [&ada, &xiaohe, &robot] {
            assert!(text.contains("Answer only by calling the `speak` tool"));
            assert!(text.contains("call `list_activities`; never guess"));
            assert!(text.contains("Never state a fact you have not seen"));
            assert!(text.contains("You do no work yourself"));
        }
        assert!(!ada.contains("likes to be called"), "no form of address picked");
        let boss = Wearing { character: Some("ada".to_owned()), address: Some("boss".to_owned()) };
        assert!(instructions(&Persona::of(&boss), None, Brain::Codex).contains("likes to be called \"boss\""));
        let laoban = Wearing { character: Some("wanwanxiaohe".to_owned()), address: Some("laoban".to_owned()) };
        assert!(instructions(&Persona::of(&laoban), None, Brain::Codex).contains("likes to be called \"老板\""));
    }

    #[tokio::test]
    async fn a_new_character_resumes_the_same_thread_with_its_persona_and_a_note() {
        let dir = temp_dir("character");
        let script = logged_in();
        let resident = resident(&dir, &script, Arc::default());
        let received = |method: &str| -> Vec<Value> {
            let script = script.lock().unwrap();
            script.received.iter().filter(|message| message["method"] == method).map(|message| message["params"].clone()).collect()
        };
        let texts = |input: &Value| -> Vec<String> {
            input["input"].as_array().unwrap().iter().map(|item| item["text"].as_str().unwrap().to_owned()).collect()
        };

        resident.say("hi", &wearing("ada")).await.unwrap();
        resident.say("again", &wearing("ada")).await.unwrap();
        assert_eq!(script.lock().unwrap().starts, 1, "the same Character keeps the running thread");
        let start = &received("thread/start")[0];
        assert!(start["developerInstructions"].as_str().unwrap().contains("Right now you are Ada"));

        resident.say("你好", &wearing("wanwanxiaohe")).await.unwrap();
        assert_eq!(script.lock().unwrap().starts, 2, "a new Character starts the app-server again");
        let resume = &received("thread/resume")[0];
        assert_eq!(resume["threadId"], "new-thread", "the same thread, never a fork");
        let instructions = resume["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("Right now you are Xiaohe (小何)") && instructions.contains("always reply in Chinese"));
        assert_eq!(received("thread/start").len(), 1, "never a new thread");
        let turns = received("turn/start");
        assert_eq!(texts(&turns[0]), ["hi"], "the first command has nothing to switch from");
        assert_eq!(texts(&turns[1]), ["again"]);
        let switched = texts(&turns[2]);
        assert!(switched[0].starts_with(CONTEXT_PREFIX) && switched[0].contains("from now on you are Xiaohe (小何)"), "{switched:?}");
        assert_eq!(switched[1], "你好");

        resident.say("还在吗", &wearing("wanwanxiaohe")).await.unwrap();
        assert_eq!(script.lock().unwrap().starts, 2);
        assert_eq!(texts(&received("turn/start")[3]), ["还在吗"], "the note goes once");

        // Whom the thread spoke as is kept, so a change while the daemon was down is noticed too.
        let restarted = self::resident(&dir, &script, Arc::default());
        restarted.say("hello", &wearing("ada")).await.unwrap();
        let after = texts(&received("turn/start")[4]);
        assert!(after[0].contains("from now on you are Ada"), "{after:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_history_leaves_out_context_notes() {
        let turn = json!({"items": [
            {"type": "userMessage", "content": [
                {"type": "text", "text": "<context>The user just switched you to another Character: from now on you are Ada.</context>"},
                {"type": "text", "text": "what's up?"},
            ]},
            {"type": "dynamicToolCall", "tool": "speak", "arguments": {"text": "All quiet."}},
        ]});
        assert_eq!(
            exchange_of(&turn),
            Some(Exchange { said: "what's up?".to_owned(), replies: vec!["All quiet.".to_owned()] })
        );
    }

    #[tokio::test]
    async fn the_history_is_read_from_the_thread_oldest_first() {
        let dir = temp_dir("history");
        let script = logged_in();
        let resident = resident(&dir, &script, Arc::default());
        assert_eq!(resident.history(5).await.unwrap(), [], "no thread yet, so nothing is started");
        assert_eq!(script.lock().unwrap().starts, 0);
        resident.say("hi", &Wearing::default()).await.unwrap();
        let history = resident.history(2).await.unwrap();
        assert_eq!(
            history,
            [
                Exchange { said: "what's up?".to_owned(), replies: vec!["One task.".to_owned(), "It's fine.".to_owned()] },
                Exchange { said: "and now?".to_owned(), replies: vec!["Still one.".to_owned()] },
            ]
        );
        let list = script.lock().unwrap().received.last().unwrap()["params"].clone();
        assert_eq!(list, json!({"threadId": "new-thread", "limit": 2, "sortDirection": "desc", "itemsView": "full"}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_is_checked_without_a_turn_cached_and_follows_what_a_start_finds() {
        let dir = temp_dir("readiness");
        let script = Arc::new(std::sync::Mutex::new(Script::default()));
        let resident = resident(&dir, &script, Arc::default());
        let mut watcher = resident.watch_ready();
        assert!(!resident.ready(), "not ready before the first check");

        assert_eq!(resident.check_readiness().await.problem, None);
        assert!(resident.ready());
        assert!(watcher.has_changed().unwrap(), "watchers hear it come on");
        watcher.mark_unchanged();
        assert_eq!(script.lock().unwrap().starts, 0, "the check starts no app-server");

        // A check younger than the TTL stands.
        *resident.fake_problem.lock().unwrap() = Some(Problem::CodexLoggedOut);
        assert!(resident.check_readiness().await.ready());
        tokio::time::advance(READINESS_TTL).await;
        assert_eq!(resident.check_readiness().await.problem, Some(Problem::CodexLoggedOut));
        assert!(!resident.ready());
        assert!(watcher.has_changed().unwrap(), "and go off");
        watcher.mark_unchanged();

        // A start that finds the user logged in is the newer word, and so is one that finds them logged out.
        script.lock().unwrap().logged_in = true;
        resident.say("hi", &Wearing::default()).await.unwrap();
        assert!(resident.ready());
        let script = Arc::new(std::sync::Mutex::new(Script::default()));
        let logged_out = super::tests::resident(&dir, &script, Arc::default());
        assert!(logged_out.check_readiness().await.ready());
        let watcher = logged_out.watch_ready();
        assert_eq!(logged_out.say("hi", &Wearing::default()).await.unwrap_err(), ResidentError::NotLoggedIn);
        assert_eq!(logged_out.readiness().problem, Some(Problem::CodexLoggedOut));
        assert!(watcher.has_changed().unwrap());

        let off = Resident::new(false, &dir, Launcher::Codex, Arc::default());
        assert!(!off.check_readiness().await.ready() && !off.ready(), "switched off, nothing is checked");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_revision_moves_when_a_turn_ends_or_the_thread_changes_but_not_on_reads() {
        let dir = temp_dir("revision");
        let saved = Saved { thread_id: Some("old-thread".to_owned()), codex_version: None, persona: None };
        save(&dir.join("resident.json"), &saved).unwrap();
        let script = logged_in();
        let first = resident(&dir, &script, Arc::default());
        assert_eq!(first.revision(), 0);
        first.history(5).await.unwrap();
        assert_eq!(first.revision(), 0, "resuming the same thread and reading it changes nothing");
        first.say("hi", &Wearing::default()).await.unwrap();
        assert_eq!(first.revision(), 1);
        first.history(5).await.unwrap();
        assert_eq!(first.revision(), 1);

        // A lost thread replaced by a new one counts, and so does a turn that fails.
        let script = logged_in();
        script.lock().unwrap().lose_thread = true;
        script.lock().unwrap().die_on_turn = Some(1);
        let second = resident(&dir, &script, Arc::default());
        assert!(second.say("one", &Wearing::default()).await.is_err());
        assert_eq!(second.revision(), 2, "the new thread and the failed turn");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_runs_in_the_resident_codex_home() {
        let command = codex_command(Path::new("/bin/codex"), Path::new("/state/resident-codex"));
        let home = command.as_std().get_envs().find(|(name, _)| *name == "CODEX_HOME").and_then(|(_, value)| value);
        assert_eq!(home, Some(std::ffi::OsStr::new("/state/resident-codex")));
    }

    #[test]
    fn the_app_server_starts_without_plugins_apps_built_in_tools_or_the_users_mcp_servers() {
        let args = app_server_args(&tool_overrides(), &["node_repl".to_owned(), "computer-use".to_owned()]);
        assert_eq!(args.last().map(String::as_str), Some("app-server"));
        let overrides: Vec<&str> =
            args.chunks(2).filter(|pair| pair[0] == "-c").map(|pair| pair[1].as_str()).collect();
        for expected in [
            "features.plugins=false",
            "features.apps=false",
            "features.shell_tool=false",
            "features.unified_exec=false",
            "features.multi_agent=false",
            "features.memories=false",
            "features.shell_snapshot=false",
            "skills.include_instructions=false",
            "skills.bundled.enabled=false",
            "web_search=\"live\"",
            "cli_auth_credentials_store=\"file\"",
            "memories.generate_memories=false",
            "model=\"gpt-6-luna\"",
            "model_reasoning_effort=\"low\"",
            "mcp_servers.node_repl.enabled=false",
            "mcp_servers.computer-use.enabled=false",
        ] {
            assert!(overrides.contains(&expected), "missing {expected}");
        }
        // Hooks stay on: the daemon filters them.
        assert!(!args.iter().any(|arg| arg.contains("hooks")));
        assert!(!args.iter().any(|arg| arg == "--disable"));
    }

    #[test]
    fn the_brain_setting_decides_readiness_and_auto_prefers_a_ready_codex() {
        let now = Some(tokio::time::Instant::now());
        let codex_ready = Readiness { checked: now, problem: None, brain: None };
        let logged_out = Readiness { checked: now, problem: Some(Problem::CodexLoggedOut), brain: None };
        let missing = Readiness { checked: now, problem: Some(Problem::CodexMissing), brain: None };
        let unchecked = Readiness::default();
        let brain = |readiness: Readiness| readiness.ready().then_some(readiness.brain).flatten();
        let problem = |readiness: Readiness| readiness.problem;

        // auto: Codex when ready, else Claude Code when it is there with a key.
        assert_eq!(brain(decide(BrainChoice::Auto, codex_ready, true, true)), Some(Brain::Codex));
        assert_eq!(brain(decide(BrainChoice::Auto, logged_out, true, true)), Some(Brain::Claude));
        assert_eq!(brain(decide(BrainChoice::Auto, unchecked, true, true)), Some(Brain::Claude));
        assert!(!decide(BrainChoice::Auto, unchecked, true, false).ready(), "nothing known yet");
        assert_eq!(decide(BrainChoice::Auto, unchecked, true, false).problem, None);
        // Not ready: without a key it is Codex's problem; with one, Claude Code's.
        assert_eq!(problem(decide(BrainChoice::Auto, missing, true, false)), Some(Problem::CodexMissing));
        assert_eq!(problem(decide(BrainChoice::Auto, logged_out, true, false)), Some(Problem::CodexLoggedOut));
        assert_eq!(problem(decide(BrainChoice::Auto, logged_out, false, true)), Some(Problem::ClaudeMissing));
        assert_eq!(decide(BrainChoice::Auto, logged_out, false, true).brain, None);

        // codex and claude stick to their brain.
        assert_eq!(brain(decide(BrainChoice::Codex, codex_ready, true, true)), Some(Brain::Codex));
        assert_eq!(problem(decide(BrainChoice::Codex, logged_out, true, true)), Some(Problem::CodexLoggedOut));
        assert_eq!(brain(decide(BrainChoice::Claude, codex_ready, true, true)), Some(Brain::Claude));
        assert_eq!(problem(decide(BrainChoice::Claude, codex_ready, false, true)), Some(Problem::ClaudeMissing));
        assert_eq!(problem(decide(BrainChoice::Claude, codex_ready, true, false)), Some(Problem::AnthropicKeyMissing));
    }

    #[tokio::test]
    async fn the_key_and_brain_are_kept_in_a_private_file_and_switching_brains_moves_the_revision() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("settings");
        let file = dir.join("config").join(SETTINGS_FILE);
        let script = Arc::new(std::sync::Mutex::new(Script::default()));
        let claude = crate::resident_claude::tests::script();
        let resident = resident(&dir, &script, Arc::default())
            .with_claude(crate::resident_claude::tests::brain(&dir, &claude, Arc::default()))
            .with_settings_file(file.clone());
        *resident.fake_problem.lock().unwrap() = Some(Problem::CodexLoggedOut);
        assert_eq!(resident.check_readiness().await.problem, Some(Problem::CodexLoggedOut));
        assert_eq!(resident.brain(), Brain::Codex);

        resident.set_anthropic_key(Some("  sk-ant-secret \n".to_owned())).await.unwrap();
        assert!(resident.ready() && resident.anthropic_key_set());
        assert_eq!(resident.brain(), Brain::Claude, "auto falls back to Claude Code with a key");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored, json!({"brain": "auto", "anthropic_api_key": "sk-ant-secret"}));

        // The voice command goes to Claude Code, with the stored key, and its session is the one reported.
        let answer = resident.say("hi", &Wearing::default()).await.unwrap();
        assert_eq!(Some(answer.thread_id.clone()), resident.thread_id());
        let launches = claude.lock().unwrap().launches.clone();
        assert!(launches[0].1.contains(&("ANTHROPIC_API_KEY".to_owned(), "sk-ant-secret".to_owned())));
        assert_eq!(script.lock().unwrap().starts, 0, "Codex wasn't started");

        // Codex comes back: auto goes back to it, and the history to show is another one.
        let before = resident.revision();
        *resident.fake_problem.lock().unwrap() = None;
        resident.set_brain(BrainChoice::Auto).await.unwrap();
        assert_eq!(resident.brain(), Brain::Codex);
        assert_eq!(resident.revision(), before + 1);

        resident.set_brain(BrainChoice::Claude).await.unwrap();
        assert_eq!(resident.brain(), Brain::Claude);
        resident.set_anthropic_key(None).await.unwrap();
        assert_eq!(resident.readiness().problem, Some(Problem::AnthropicKeyMissing));
        assert!(!resident.ready());
        assert_eq!(resident.say("hi", &Wearing::default()).await.unwrap_err(), ResidentError::NoAnthropicKey);
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored, json!({"brain": "claude"}), "the key is gone from the file");

        // Read back at the next start.
        let again = Resident::new(true, &dir, Launcher::Codex, Arc::default()).with_settings_file(file);
        assert_eq!(again.brain_setting(), BrainChoice::Claude);
        assert!(!again.anthropic_key_set());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_claude_history_and_hooks_are_the_claude_sessions() {
        let dir = temp_dir("claude-hooks");
        let claude = crate::resident_claude::tests::script();
        let resident = resident(&dir, &logged_in(), Arc::default())
            .with_claude(crate::resident_claude::tests::brain(&dir, &claude, Arc::default()));
        resident.set_anthropic_key(Some("key".to_owned())).await.unwrap();
        resident.set_brain(BrainChoice::Claude).await.unwrap();
        assert_eq!(resident.history(5).await.unwrap(), [], "no session yet");
        resident.say("hi", &Wearing::default()).await.unwrap();
        let session = resident.thread_id().unwrap();
        assert!(resident.owns_claude_hook(&session, None));
        assert!(resident.owns_claude_hook("other", Some(&dir.join("resident").display().to_string())));
        assert!(!resident.owns_claude_hook("other", Some("/work/vibe-buddy")));
        assert!(!resident.owns_hook(&session, None, None), "a Claude session id isn't a Codex thread");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hooks_from_the_resident_thread_or_its_directory_are_recognised() {
        let dir = temp_dir("hooks");
        let saved = Saved { thread_id: Some("resident-thread".to_owned()), codex_version: None, persona: None };
        save(&dir.join("resident.json"), &saved).unwrap();
        let resident = Resident::new(false, &dir, Launcher::Codex, Arc::default());
        let cwd = dir.join("resident").display().to_string();
        assert!(resident.owns_hook("resident-thread", None, None));
        assert!(resident.owns_hook("subagent", Some("resident-thread"), None));
        assert!(resident.owns_hook("other", None, Some(&cwd)));
        assert!(resident.owns_hook("other", None, Some(&format!("{cwd}/notes"))));
        assert!(!resident.owns_hook("other", Some("other"), Some("/work/vibe-buddy")));
        assert!(!resident.owns_hook("other", None, Some(&format!("{cwd}-not"))));
        assert!(!resident.owns_hook("other", None, None));
        let _ = std::fs::remove_dir_all(dir);
    }
}
