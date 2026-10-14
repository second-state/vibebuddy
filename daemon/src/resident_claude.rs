//! The resident session on Claude Code (ADR-0017): the user's own `claude` CLI, run headless with the user's own
//! Anthropic API key, never a claude.ai login (Anthropic doesn't let third-party products use one).
//!
//! One long-lived `claude -p --input-format stream-json --output-format stream-json` process holds the resident
//! session; each voice command is one user message on its stdin, and the turn is over at the `result` line. The
//! session is created once with `--session-id` and taken up again with `--resume` after every restart, so like the
//! Codex thread it is one session for good, replaced only when its transcript is gone.
//!
//! - **Its own config dir.** `CLAUDE_CONFIG_DIR` is `resident-claude/` in the daemon's state directory (0700), so
//!   none of the user's Claude config loads: settings, CLAUDE.md, hooks, plugins, MCP servers, and their login (the
//!   credentials file and the macOS Keychain entry are keyed by the config dir).
//! - **Only the user's API key.** The child's environment loses every `CLAUDE*` and `ANTHROPIC*` variable the daemon
//!   has (an OAuth token, an auth token, a provider switch) and gets `ANTHROPIC_API_KEY`, which `-p` always uses when
//!   present. The `system/init` line says where the key came from; anything but `ANTHROPIC_API_KEY` stops the
//!   process. The key is never logged.
//! - **Only our tools.** `--tools WebSearch` leaves Claude Code's only built-in tool the web search; `speak` and
//!   `list_activities` come from an MCP server the daemon runs on a loopback port of its own, behind a random
//!   token (`--strict-mcp-config`, so no other server loads). `--permission-mode dontAsk` with only those allowed,
//!   and the usual built-ins denied by name besides. `--bare` would skip even more, but it also takes WebSearch
//!   away (checked with claude 2.1.296), so the config dir and the flags do it instead.
//!
//! Experimental, behind the same `VIBEBUDDY_RESIDENT_SESSION=1` as the Codex session; see docs/resident-session.md.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::sync::{Mutex, OnceCell, mpsc};
use tracing::{info, warn};

use crate::activity::ActivityTracker;
use crate::persona::Persona;
use crate::resident::{Exchange, Pipes, ResidentError, change_note, run_tool, tools};

/// The model the resident session runs on: a voice command waits on the answer.
pub const MODEL: &str = "claude-haiku-4-5";
/// The Claude Code version the flags and the stream were checked with; another one gets a warning and is tried.
pub const TESTED_CLAUDE_VERSION: &str = "2.1.296";
/// Names the `claude` binary explicitly; otherwise it is looked for on PATH and where installers put it.
pub const CLAUDE_VARIABLE: &str = "VIBEBUDDY_CLAUDE";
/// Development only: where the child sends its API requests (a fake Messages API in testing). The daemon's own
/// `ANTHROPIC_BASE_URL`, like every other `ANTHROPIC*` variable, never reaches the child.
pub const BASE_URL_VARIABLE: &str = "VIBEBUDDY_CLAUDE_BASE_URL";
/// The resident session's own Claude config dir, in the daemon's state directory.
const CONFIG_DIR: &str = "resident-claude";
/// The session id and the Claude Code version it was last started with, in the daemon's state directory.
const STATE_FILE: &str = "resident-claude.json";
/// The name the daemon's MCP server has in Claude Code: its tools are `mcp__vibebuddy__<tool>`.
const MCP_SERVER: &str = "vibebuddy";
/// The one built-in tool the session gets.
const WEB_SEARCH: &str = "WebSearch";
/// Built-in tools denied by name on top of `--tools`, so a Claude Code that changes what `--tools` covers still
/// gives the session none of them.
const DENIED_TOOLS: [&str; 12] =
    ["Bash", "Edit", "Write", "Read", "NotebookEdit", "WebFetch", "Glob", "Grep", "Agent", "Task", "Skill", "BashOutput"];
/// How long one voice command may take, from writing it to the `result` line; then the process is stopped.
const TURN_TIMEOUT: Duration = Duration::from_secs(120);
/// What the `system/init` line must report as the credential in use.
const API_KEY_SOURCE: &str = "ANTHROPIC_API_KEY";
/// Starts a voice command's context block, which carries the local time; the history leaves it out.
pub(crate) const CONTEXT_PREFIX: &str = "<context>";

/// How `claude` is reached: the real binary, or an in-memory fake in tests.
pub enum Launcher {
    Claude,
    #[cfg(test)]
    Fake(Box<dyn Fn(&Launch) -> Pipes + Send + Sync>),
}

/// What would be run: the arguments, and the variables set on top of the daemon's cleaned environment.
pub struct Launch {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

struct Process {
    writer: Box<dyn AsyncWrite + Send + Unpin>,
    lines: Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
    /// The key it was started with: a new key needs a new process.
    key: String,
    /// The instructions it was started with: `--system-prompt` is per process, so new ones (a new Character) need
    /// a new process on the same session.
    instructions: String,
    _child: Option<tokio::process::Child>,
}

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    session_id: Option<String>,
    /// The Claude Code version the session was last started with, for diagnosing a change in the stream.
    claude_version: Option<String>,
    /// The Character whose persona the session last answered in.
    #[serde(default)]
    persona: Option<String>,
}

/// What the daemon's MCP server shares with the turn in progress.
struct McpState {
    token: String,
    activities: Arc<Mutex<ActivityTracker>>,
    /// Where `speak` goes while a turn runs; None between turns.
    speak: std::sync::Mutex<Option<mpsc::UnboundedSender<String>>>,
}

pub struct ClaudeBrain {
    /// The working directory, shared with the Codex session: `resident/` in the state directory.
    dir: PathBuf,
    config_dir: PathBuf,
    state_file: PathBuf,
    launcher: Launcher,
    mcp: Arc<McpState>,
    /// The loopback address of the MCP server, once started.
    mcp_url: OnceCell<String>,
    /// Held for a whole turn: one voice command at a time.
    process: Mutex<Option<Process>>,
    /// Read synchronously by the hook filter.
    session_id: std::sync::Mutex<Option<String>>,
    /// The Character whose persona the session last answered in (kept in the state file).
    spoke_as: std::sync::Mutex<Option<String>>,
}

impl ClaudeBrain {
    pub fn new(state_dir: &Path, launcher: Launcher, activities: Arc<Mutex<ActivityTracker>>) -> Self {
        let state_file = state_dir.join(STATE_FILE);
        let saved: Saved = std::fs::read_to_string(&state_file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            dir: state_dir.join("resident"),
            config_dir: state_dir.join(CONFIG_DIR),
            state_file,
            launcher,
            mcp: Arc::new(McpState { token: random_hex(32), activities, speak: std::sync::Mutex::new(None) }),
            mcp_url: OnceCell::new(),
            process: Mutex::new(None),
            session_id: std::sync::Mutex::new(saved.session_id),
            spoke_as: std::sync::Mutex::new(saved.persona),
        }
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().expect("session id lock").clone()
    }

    /// Whether `claude` can be found; the readiness check, which starts nothing.
    pub fn installed(&self) -> bool {
        match &self.launcher {
            Launcher::Claude => find_claude().is_some(),
            #[cfg(test)]
            Launcher::Fake(_) => true,
        }
    }

    /// Stops the process, if any, so the next command starts one with the current key.
    pub async fn stop(&self) {
        *self.process.lock().await = None;
    }

    /// Starts `claude` ahead of the first voice command. A failure is logged; the next command tries again.
    pub async fn warm(&self, key: &str, instructions: &str) {
        let mut slot = self.process.lock().await;
        if let Err(error) = self.running(&mut slot, key, instructions).await {
            warn!(%error, "the resident Claude session could not be started ahead of time");
        }
    }

    /// Hands one voice command to the session, answered in `instructions`, which make the buddy `persona`; each
    /// spoken reply goes to `on_reply` when `speak` is called. Returns the session id once the turn is over.
    pub async fn say_streaming(
        &self,
        key: &str,
        instructions: &str,
        persona: &Persona,
        text: &str,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        let mut slot = self.process.lock().await;
        let process = self.running(&mut slot, key, instructions).await?;
        let spoke_as = self.spoke_as.lock().expect("spoke as lock").clone();
        let changed = spoke_as.as_deref() != Some(persona.id);
        let note = (changed && spoke_as.is_some()).then(|| change_note(persona));
        let (sender, spoken) = mpsc::unbounded_channel();
        *self.mcp.speak.lock().expect("speak lock") = Some(sender);
        let result = run_turn(process, spoken, persona, note.as_deref(), text, on_reply).await;
        *self.mcp.speak.lock().expect("speak lock") = None;
        match &result {
            // The process is still there and the session intact: an API error, a refused key.
            Err(ResidentError::TurnEnded(_)) | Ok(()) => {}
            Err(_) => *slot = None,
        }
        if result.is_ok() && changed {
            self.remember_persona(persona.id);
        }
        result.map(|()| self.session_id().unwrap_or_default())
    }

    /// Keeps the Character the session now answers as, so a change is noticed after a restart too.
    fn remember_persona(&self, id: &str) {
        *self.spoke_as.lock().expect("spoke as lock") = Some(id.to_owned());
        let mut saved: Saved = std::fs::read_to_string(&self.state_file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        saved.persona = Some(id.to_owned());
        if let Err(error) = write_private(&self.state_file, &serde_json::to_string_pretty(&saved).unwrap_or_default()) {
            warn!(%error, "failed to save the resident Claude session's Character");
        }
    }

    /// The last `limit` voice commands and their spoken replies, oldest first, read from the session's transcript
    /// (the daemon keeps no record of its own). Empty while there is no session yet.
    pub fn history(&self, limit: usize) -> Vec<Exchange> {
        let Some(path) = self.session_id().and_then(|id| transcript(&self.config_dir, &id)) else {
            return Vec::new();
        };
        let exchanges = exchanges_of(&std::fs::read_to_string(path).unwrap_or_default());
        let skip = exchanges.len().saturating_sub(limit);
        exchanges.into_iter().skip(skip).collect()
    }

    /// Whether a Claude Code hook event comes from the resident session: by its session id. (The caller checks the
    /// working directory.)
    pub fn owns_session(&self, session_id: &str) -> bool {
        self.session_id.lock().expect("session id lock").as_deref() == Some(session_id)
    }

    async fn running<'a>(
        &self,
        slot: &'a mut Option<Process>,
        key: &str,
        instructions: &str,
    ) -> Result<&'a mut Process, ResidentError> {
        if slot.as_ref().is_some_and(|process| process.key != key) {
            info!("the Anthropic API key changed; starting the resident Claude session again");
            *slot = None;
        }
        if slot.as_ref().is_some_and(|process| process.instructions != instructions) {
            info!("the instructions changed (a new Character); resuming the resident Claude session with them");
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(self.start(key, instructions).await?);
        }
        Ok(slot.as_mut().expect("process was just started"))
    }

    async fn start(&self, key: &str, instructions: &str) -> Result<Process, ResidentError> {
        let failed = |what: &str, error: std::io::Error| ResidentError::Rpc(format!("cannot {what}: {error}"));
        prepare_config_dir(&self.config_dir).map_err(|error| failed("prepare the Claude config dir", error))?;
        std::fs::create_dir_all(&self.dir).map_err(|error| failed("create the resident directory", error))?;
        let url = self.mcp_url.get_or_try_init(|| serve_mcp(self.mcp.clone())).await.map_err(|error| failed("start the MCP server", error))?;
        let mcp_config = self.config_dir.join("mcp.json");
        write_private(&mcp_config, &mcp_config_json(url, &self.mcp.token).to_string())
            .map_err(|error| failed("write the MCP config", error))?;
        // One session for good: resumed while its transcript is there, else a new one.
        let saved = self.session_id();
        let session = match &saved {
            Some(id) if transcript(&self.config_dir, id).is_some() => Session::Resume(id.clone()),
            other => {
                let id = new_session_id();
                if let Some(old) = other {
                    warn!(old_session = %old, new_session = %id, "the resident Claude session's transcript is gone; starting a new one");
                }
                Session::New(id)
            }
        };
        let launch = Launch {
            args: claude_args(&mcp_config, &session, instructions),
            env: child_env(&self.config_dir, key, std::env::var(BASE_URL_VARIABLE).ok()),
        };
        let (pipes, version) = match &self.launcher {
            Launcher::Claude => spawn_claude(&launch, &self.dir).await?,
            #[cfg(test)]
            Launcher::Fake(connect) => (connect(&launch), None),
        };
        let id = session.id().to_owned();
        *self.session_id.lock().expect("session id lock") = Some(id.clone());
        let spoke_as = self.spoke_as.lock().expect("spoke as lock").clone();
        let saved = Saved { session_id: Some(id.clone()), claude_version: version, persona: spoke_as };
        if let Err(error) = write_private(&self.state_file, &serde_json::to_string_pretty(&saved).unwrap_or_default()) {
            warn!(%error, "failed to save the resident Claude session id");
        }
        info!(session = %id, resumed = matches!(session, Session::Resume(_)), "resident Claude session started");
        Ok(Process {
            writer: pipes.writer,
            lines: BufReader::new(pipes.reader).lines(),
            key: key.to_owned(),
            instructions: instructions.to_owned(),
            _child: pipes.child,
        })
    }
}

enum Session {
    New(String),
    Resume(String),
}

impl Session {
    fn id(&self) -> &str {
        match self {
            Self::New(id) | Self::Resume(id) => id,
        }
    }
}

/// One turn: the voice command in, lines out until `result`, and whatever `speak` said meanwhile.
async fn run_turn(
    process: &mut Process,
    mut spoken: mpsc::UnboundedReceiver<String>,
    persona: &Persona,
    note: Option<&str>,
    text: &str,
    on_reply: &mut (dyn FnMut(&str) + Send),
) -> Result<(), ResidentError> {
    let line = format!("{}\n", user_message(text, persona, note, &chrono::Local::now(), local_time_zone().as_deref()));
    let written = async {
        process.writer.write_all(line.as_bytes()).await?;
        process.writer.flush().await
    };
    written.await.map_err(|error| ResidentError::Rpc(format!("claude is gone: {error}")))?;
    let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
    loop {
        tokio::select! {
            Some(reply) = spoken.recv() => on_reply(&reply),
            line = tokio::time::timeout_at(deadline, process.lines.next_line()) => {
                let line = match line {
                    Err(_) => return Err(ResidentError::Timeout),
                    Ok(Ok(Some(line))) => line,
                    Ok(_) => return Err(ResidentError::Rpc("claude exited".to_owned())),
                };
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                match (message["type"].as_str(), message["subtype"].as_str()) {
                    (Some("system"), Some("init")) => {
                        let source = message["apiKeySource"].as_str().unwrap_or("none");
                        if source != API_KEY_SOURCE {
                            warn!(source, "the resident Claude session isn't using the user's API key; stopped it");
                            return Err(ResidentError::Rpc(format!(
                                "Claude Code would authenticate with {source}, not the Anthropic API key; refused"
                            )));
                        }
                    }
                    (Some("result"), _) => {
                        // `speak` is answered before Claude Code goes on, so anything it said is already queued.
                        while let Ok(reply) = spoken.try_recv() {
                            on_reply(&reply);
                        }
                        if message["is_error"].as_bool() == Some(true) {
                            let reason = message["result"]
                                .as_str()
                                .or(message["terminal_reason"].as_str())
                                .or(message["subtype"].as_str())
                                .unwrap_or("unknown");
                            return Err(ResidentError::TurnEnded(reason.to_owned()));
                        }
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The computer's IANA time zone (`Asia/Singapore`): `TZ` if set, else where `/etc/localtime` points, on macOS
/// and Linux alike.
fn local_time_zone() -> Option<String> {
    if let Ok(zone) = std::env::var("TZ")
        && !zone.is_empty()
    {
        return Some(zone.trim_start_matches(':').to_owned());
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let path = target.to_string_lossy();
    path.split_once("zoneinfo/").map(|(_, zone)| zone.to_owned())
}

/// A voice command as a stream-json user message: the local time and time zone first, as context, then what was
/// said. The time zone stands in for where the user is (Codex gets it from its own environment context), so a
/// question about the weather or the news has a place. A `note` (the Character changed) goes with the context.
/// Every command also repeats who answers and in which language: with only the system prompt saying so, Haiku
/// answered a Chinese question in Chinese as an English Character, carried by the user's language and the session's
/// earlier replies.
fn user_message(
    text: &str,
    persona: &Persona,
    note: Option<&str>,
    now: &chrono::DateTime<chrono::Local>,
    time_zone: Option<&str>,
) -> Value {
    let zone = time_zone.map(|zone| format!(", time zone {zone}")).unwrap_or_default();
    let Persona { name, language, .. } = persona;
    let context = format!(
        "{CONTEXT_PREFIX}The user's local time: {}{zone}. You are {name}: answer in {language} only, whatever language \
         this is said in.</context>",
        now.format("%A %Y-%m-%d %H:%M (UTC%:z)")
    );
    let content: Vec<Value> =
        [context.as_str()].into_iter().chain(note).chain([text]).map(|text| json!({"type": "text", "text": text})).collect();
    json!({"type": "user", "message": {"role": "user", "content": content}})
}

/// The arguments for `claude`; see the module docs for why each is there.
fn claude_args(mcp_config: &Path, session: &Session, instructions: &str) -> Vec<String> {
    let allowed: Vec<String> = std::iter::once(WEB_SEARCH.to_owned())
        .chain(mcp_tools().iter().map(|tool| format!("mcp__{MCP_SERVER}__{}", tool["name"].as_str().unwrap_or_default())))
        .collect();
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--model",
        MODEL,
        "--tools",
        WEB_SEARCH,
        "--permission-mode",
        "dontAsk",
        "--permission-prompts",
        "none",
        "--strict-mcp-config",
        "--disable-slash-commands",
        // The instructions are read again on every request, so a change reaches a resumed session.
        "--system-prompt-snapshot",
        "off",
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(["--allowedTools".to_owned(), allowed.join(",")]);
    args.extend(["--disallowedTools".to_owned(), DENIED_TOOLS.join(",")]);
    args.extend(["--mcp-config".to_owned(), mcp_config.display().to_string()]);
    args.extend(["--settings".to_owned(), json!({"disableAllHooks": true}).to_string()]);
    // Replaces Claude Code's coding-assistant prompt: the buddy isn't one, and a short prompt is a faster turn.
    args.extend(["--system-prompt".to_owned(), instructions.to_owned()]);
    match session {
        Session::New(id) => args.extend(["--session-id".to_owned(), id.clone()]),
        Session::Resume(id) => args.extend(["--resume".to_owned(), id.clone()]),
    }
    args
}

/// What the child gets on top of the daemon's environment, which loses every `CLAUDE*` and `ANTHROPIC*` variable
/// first (see [`spawn_claude`]).
fn child_env(config_dir: &Path, key: &str, base_url: Option<String>) -> Vec<(String, String)> {
    let mut env = vec![
        ("CLAUDE_CONFIG_DIR".to_owned(), config_dir.display().to_string()),
        ("ANTHROPIC_API_KEY".to_owned(), key.to_owned()),
        // No memory files written into the config dir for a session that has its own way to remember.
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_owned(), "1".to_owned()),
        // The daemon's child doesn't update the user's Claude Code behind their back.
        ("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned()),
    ];
    if let Some(url) = base_url.filter(|url| !url.is_empty()) {
        env.push(("ANTHROPIC_BASE_URL".to_owned(), url));
    }
    env
}

/// Whether a variable of the daemon's environment is kept from the child: every one that can choose Claude Code's
/// credential or provider (an OAuth token, an auth token, Bedrock/Vertex/Foundry switches, profiles) starts so.
fn withheld(name: &str) -> bool {
    name.starts_with("CLAUDE") || name.starts_with("ANTHROPIC")
}

async fn spawn_claude(launch: &Launch, cwd: &Path) -> Result<(Pipes, Option<String>), ResidentError> {
    let program = find_claude().ok_or(ResidentError::NoClaude)?;
    let command = |args: &[String]| {
        let mut command = tokio::process::Command::new(&program);
        for (name, _) in std::env::vars_os() {
            if withheld(&name.to_string_lossy()) {
                command.env_remove(&name);
            }
        }
        command.envs(launch.env.iter().map(|(name, value)| (name, value))).args(args).current_dir(cwd);
        command
    };
    let version = command(&["--version".to_owned()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.split_whitespace().next().map(str::to_owned));
    if version.as_deref() != Some(TESTED_CLAUDE_VERSION) {
        warn!(
            version = version.as_deref().unwrap_or("unknown"),
            tested = TESTED_CLAUDE_VERSION,
            "the resident session was tested with another Claude Code version; its flags or stream may differ"
        );
    }
    let mut child = command(&launch.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| ResidentError::Rpc(format!("cannot start {}: {error}", program.display())))?;
    info!(program = %program.display(), "started claude for the resident session");
    // A refused flag or a crash is only on stderr; it carries no credentials.
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let end = line.floor_char_boundary(300);
                info!(line = &line[..end], "claude stderr");
            }
        });
    }
    let reader = child.stdout.take().expect("stdout is piped");
    let writer = child.stdin.take().expect("stdin is piped");
    Ok((Pipes { reader: Box::new(reader), writer: Box::new(writer), child: Some(child) }, version))
}

/// `VIBEBUDDY_CLAUDE`, else `claude` on PATH, else where installers put it: the daemon may run with launchd's bare
/// PATH.
pub fn find_claude() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(CLAUDE_VARIABLE) {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|dir| dir.join("claude")).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(home.iter().flat_map(|home| [home.join(".local/bin/claude"), home.join(".claude/local/claude")]))
        .chain(["/opt/homebrew/bin/claude", "/usr/local/bin/claude"].map(PathBuf::from))
        .find(|path| path.is_file())
}

fn prepare_config_dir(config_dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(config_dir)?;
    std::fs::set_permissions(config_dir, std::fs::Permissions::from_mode(0o700))
}

/// Writes a file only the user can read (0600), replacing it whole.
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary)?;
    // A file left from before with wider permissions keeps them through `open`.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(temporary, path)
}

fn mcp_config_json(url: &str, token: &str) -> Value {
    json!({"mcpServers": {MCP_SERVER: {"type": "http", "url": url, "headers": {"Authorization": format!("Bearer {token}")}}}})
}

/// The session's transcript in the config dir (`projects/<the working directory>/<id>.jsonl`), if it exists.
fn transcript(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    std::fs::read_dir(config_dir.join("projects"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(format!("{session_id}.jsonl")))
        .find(|path| path.is_file())
}

/// The voice commands in a transcript and what was said through `speak` after each, oldest first. Lines are put
/// in time order first: Claude Code doesn't always write them in order.
fn exchanges_of(transcript: &str) -> Vec<Exchange> {
    let speak = format!("mcp__{MCP_SERVER}__speak");
    let mut entries: Vec<Value> = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| matches!(entry["type"].as_str(), Some("user" | "assistant")) && entry["isSidechain"] != true)
        .collect();
    entries.sort_by(|a, b| a["timestamp"].as_str().unwrap_or_default().cmp(b["timestamp"].as_str().unwrap_or_default()));
    let mut exchanges: Vec<Exchange> = Vec::new();
    for entry in entries {
        let content = &entry["message"]["content"];
        if entry["type"] == "user" {
            if entry["isMeta"] == true || entry["isCompactSummary"] == true {
                continue;
            }
            let said = match content {
                Value::String(text) => text.clone(),
                Value::Array(blocks) => blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .filter(|text| !text.starts_with(CONTEXT_PREFIX))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !said.is_empty() {
                exchanges.push(Exchange { said, replies: Vec::new() });
            }
        } else if let (Some(exchange), Some(blocks)) = (exchanges.last_mut(), content.as_array()) {
            exchange.replies.extend(
                blocks
                    .iter()
                    .filter(|block| block["type"] == "tool_use" && block["name"] == speak.as_str())
                    .filter_map(|block| block["input"]["text"].as_str().map(str::to_owned)),
            );
        }
    }
    exchanges
}

/// Our tools as MCP tools: the Codex session's, less `search_memories` (Codex memories are Codex's).
fn mcp_tools() -> Vec<Value> {
    tools()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| tool["name"] != "search_memories")
        .map(|tool| json!({"name": tool["name"], "description": tool["description"], "inputSchema": tool["inputSchema"]}))
        .collect()
}

/// Starts the MCP server on a loopback port of its own and returns its URL.
async fn serve_mcp(state: Arc<McpState>) -> std::io::Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", listener.local_addr()?);
    let router = axum::Router::new().route("/mcp", axum::routing::post(post_mcp).get(no_stream).delete(no_stream)).with_state(state);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            warn!(%error, "the resident session's MCP server stopped");
        }
    });
    info!(%url, "the resident session's MCP server is listening");
    Ok(url)
}

/// No server-sent stream: every answer comes back on its POST.
async fn no_stream() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// MCP's Streamable HTTP transport, the JSON-response form: one JSON-RPC message per POST.
async fn post_mcp(State(state): State<Arc<McpState>>, headers: HeaderMap, body: String) -> axum::response::Response {
    let authorized = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| same(token.as_bytes(), state.token.as_bytes()));
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}})))
            .into_response();
    };
    let Some(id) = message.get("id").cloned() else {
        // A notification (`notifications/initialized` and the like) needs no answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let answer = match mcp_result(&state, message["method"].as_str().unwrap_or_default(), &message["params"]).await {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, text)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}}),
    };
    axum::Json(answer).into_response()
}

async fn mcp_result(state: &McpState, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let version = params["protocolVersion"].as_str().unwrap_or("2025-06-18");
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": MCP_SERVER, "title": "VibeBuddy", "version": env!("CARGO_PKG_VERSION")},
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": mcp_tools()})),
        "tools/call" => {
            let tool = params["name"].as_str().unwrap_or_default();
            if tool == "search_memories" {
                return Ok(json!({"content": [{"type": "text", "text": "unknown tool search_memories"}], "isError": true}));
            }
            let speak = state.speak.lock().expect("speak lock").clone();
            let (success, text) = run_tool(tool, &params["arguments"], &state.activities, None, &mut |reply| match &speak {
                Some(turn) => {
                    let _ = turn.send(reply.to_owned());
                }
                None => warn!("the resident Claude session spoke outside a turn; dropped"),
            })
            .await;
            Ok(json!({"content": [{"type": "text", "text": text}], "isError": !success}))
        }
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

/// Compares two secrets in time that doesn't depend on where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |difference, (x, y)| difference | (x ^ y)) == 0
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the system's random source");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A random (version 4) UUID, as `--session-id` wants.
fn new_session_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;
    use crate::resident::tests::robot;

    #[test]
    fn a_voice_command_carries_the_local_time_and_time_zone_as_context() {
        let now = chrono::Local::now();
        let message = user_message("今天天气怎么样", &Persona::of(&crate::resident::tests::wearing("ada")), None, &now, Some("Asia/Singapore"));
        let content = message["message"]["content"].as_array().unwrap();
        let context = content[0]["text"].as_str().unwrap();
        assert!(context.starts_with(CONTEXT_PREFIX) && context.contains("time zone Asia/Singapore"), "{context}");
        assert!(context.contains("You are Ada: answer in English only"), "{context}");
        assert_eq!(content[1]["text"], "今天天气怎么样");
        let without = user_message("hi", &Persona::of(&crate::resident::tests::wearing("ada")), None, &now, None);
        assert!(!without["message"]["content"][0]["text"].as_str().unwrap().contains("time zone"));
    }
    use crate::activity::{ActivityId, ActivityStatus};
    use crate::resident::tests::temp_dir;

    /// One thing the fake `claude` does while answering a voice command.
    #[derive(Clone)]
    pub(crate) enum Step {
        /// The `system/init` line, naming where the credential came from.
        Init(&'static str),
        /// Calls one of the daemon's MCP tools over HTTP, as Claude Code would, with the token from the MCP config.
        Call(&'static str, Value),
        /// Waits for the test before going on.
        Hold,
        /// The `result` line: whether it is an error, and its text.
        Result(bool, &'static str),
        /// Ends its output, as if the process died.
        Exit,
    }

    /// How the fake `claude` behaves, and what it saw.
    #[derive(Default)]
    pub(crate) struct Script {
        /// What each voice command gets, in order; a command with none left gets an init and a result.
        pub(crate) turns: VecDeque<Vec<Step>>,
        /// The arguments and environment of each start.
        pub(crate) launches: Vec<Launched>,
        /// The lines the daemon wrote.
        pub(crate) received: Vec<Value>,
        /// The MCP server's answers to the calls made, with their HTTP status.
        pub(crate) answers: Vec<(u16, Value)>,
        pub(crate) hold: Option<Arc<tokio::sync::Notify>>,
    }

    pub(crate) type Shared = Arc<std::sync::Mutex<Script>>;
    /// The arguments and environment of one start.
    pub(crate) type Launched = (Vec<String>, Vec<(String, String)>);

    pub(crate) fn script() -> Shared {
        Arc::default()
    }

    /// A Claude brain on the fake `claude`, in `state_dir`.
    pub(crate) fn brain(state_dir: &Path, script: &Shared, activities: Arc<Mutex<ActivityTracker>>) -> ClaudeBrain {
        let script = script.clone();
        ClaudeBrain::new(state_dir, Launcher::Fake(Box::new(move |launch| fake_claude(script.clone(), launch))), activities)
    }

    fn argument(args: &[String], flag: &str) -> Option<String> {
        args.iter().position(|arg| arg == flag).and_then(|index| args.get(index + 1)).cloned()
    }

    /// Posts one JSON-RPC message to the MCP server named in the MCP config file, as Claude Code does.
    pub(crate) async fn post_to_mcp(config: &Path, token: Option<&str>, message: Value) -> (u16, Value) {
        let config: Value = serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
        let server = &config["mcpServers"][MCP_SERVER];
        let url = server["url"].as_str().unwrap().to_owned();
        let authorization = match token {
            Some(token) => format!("Bearer {token}"),
            None => server["headers"]["Authorization"].as_str().unwrap().to_owned(),
        };
        tokio::task::spawn_blocking(move || {
            let mut response = crate::speech::agent(Duration::from_secs(5))
                .post(&url)
                .header("Authorization", &authorization)
                .header("Content-Type", "application/json")
                .send(message.to_string().as_bytes())
                .unwrap();
            let status = response.status().as_u16();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&body).unwrap_or(Value::Null))
        })
        .await
        .unwrap()
    }

    fn fake_claude(script: Shared, launch: &Launch) -> Pipes {
        script.lock().unwrap().launches.push((launch.args.clone(), launch.env.clone()));
        let mcp_config = PathBuf::from(argument(&launch.args, "--mcp-config").unwrap());
        let (daemon_side, fake_side) = tokio::io::duplex(1 << 16);
        let (daemon_read, daemon_write) = tokio::io::split(daemon_side);
        tokio::spawn(async move {
            let (fake_read, mut out) = tokio::io::split(fake_side);
            let mut lines = BufReader::new(fake_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                script.lock().unwrap().received.push(serde_json::from_str(&line).unwrap());
                let steps = script.lock().unwrap().turns.pop_front();
                let steps = steps.unwrap_or_else(|| vec![Step::Init(API_KEY_SOURCE), Step::Result(false, "")]);
                for (number, step) in steps.into_iter().enumerate() {
                    let line = match step {
                        Step::Init(source) => json!({"type": "system", "subtype": "init", "apiKeySource": source, "tools": [WEB_SEARCH]}),
                        Step::Call(tool, arguments) => {
                            let name = format!("mcp__{MCP_SERVER}__{tool}");
                            let call = json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": number, "name": name, "input": arguments}]}});
                            out.write_all(format!("{call}\n").as_bytes()).await.unwrap();
                            let message = json!({"jsonrpc": "2.0", "id": number, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
                            let answer = post_to_mcp(&mcp_config, None, message).await;
                            script.lock().unwrap().answers.push(answer);
                            continue;
                        }
                        Step::Hold => {
                            let hold = script.lock().unwrap().hold.clone().unwrap();
                            hold.notified().await;
                            continue;
                        }
                        Step::Result(is_error, text) => {
                            json!({"type": "result", "subtype": "success", "is_error": is_error, "result": text})
                        }
                        Step::Exit => return,
                    };
                    out.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                }
            }
        });
        Pipes { reader: Box::new(daemon_read), writer: Box::new(daemon_write), child: None }
    }

    fn launches(script: &Shared) -> Vec<Launched> {
        script.lock().unwrap().launches.clone()
    }

    async fn say(brain: &ClaudeBrain, key: &str, text: &str) -> Result<Vec<String>, ResidentError> {
        let mut replies = Vec::new();
        brain.say_streaming(key, "You are the buddy of VibeBuddy.", &robot(), text, &mut |reply| replies.push(reply.to_owned())).await?;
        Ok(replies)
    }

    #[tokio::test]
    async fn the_first_command_starts_one_session_with_only_our_tools_and_the_users_key() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("claude-start");
        let script = script();
        let brain = brain(&dir, &script, Arc::default());
        say(&brain, "sk-ant-test", "hello").await.unwrap();

        let (args, env) = launches(&script).remove(0);
        let flags = |name: &str| args.iter().any(|arg| arg == name);
        for flag in ["-p", "--verbose", "--strict-mcp-config", "--disable-slash-commands"] {
            assert!(flags(flag), "missing {flag}: {args:?}");
        }
        assert!(!flags("--bare"), "bare mode takes WebSearch away");
        assert_eq!(argument(&args, "--input-format").as_deref(), Some("stream-json"));
        assert_eq!(argument(&args, "--output-format").as_deref(), Some("stream-json"));
        assert_eq!(argument(&args, "--model").as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(argument(&args, "--tools").as_deref(), Some("WebSearch"));
        assert_eq!(argument(&args, "--permission-mode").as_deref(), Some("dontAsk"));
        assert_eq!(argument(&args, "--permission-prompts").as_deref(), Some("none"));
        assert_eq!(argument(&args, "--system-prompt-snapshot").as_deref(), Some("off"));
        assert_eq!(
            argument(&args, "--allowedTools").as_deref(),
            Some("WebSearch,mcp__vibebuddy__speak,mcp__vibebuddy__list_activities")
        );
        let denied = argument(&args, "--disallowedTools").unwrap();
        for tool in ["Bash", "Edit", "Write", "Read", "WebFetch"] {
            assert!(denied.split(',').any(|denied| denied == tool), "{tool} isn't denied: {denied}");
        }
        assert_eq!(argument(&args, "--settings").as_deref(), Some(r#"{"disableAllHooks":true}"#));
        assert_eq!(argument(&args, "--system-prompt").as_deref(), Some("You are the buddy of VibeBuddy."));
        let session = argument(&args, "--session-id").unwrap();
        assert_eq!(session.len(), 36);
        assert_eq!(&session[14..15], "4", "a version 4 UUID");
        assert!(!flags("--resume"));

        let variable = |name: &str| env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
        let config_dir = dir.join("resident-claude");
        assert_eq!(variable("CLAUDE_CONFIG_DIR"), Some(config_dir.display().to_string()));
        assert_eq!(variable("ANTHROPIC_API_KEY").as_deref(), Some("sk-ant-test"));
        assert_eq!(variable("ANTHROPIC_BASE_URL"), None);
        assert_eq!(std::fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777, 0o700);
        let mcp_config = PathBuf::from(argument(&args, "--mcp-config").unwrap());
        assert_eq!(mcp_config, config_dir.join("mcp.json"));
        assert_eq!(std::fs::metadata(&mcp_config).unwrap().permissions().mode() & 0o777, 0o600);
        let config: Value = serde_json::from_str(&std::fs::read_to_string(&mcp_config).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["vibebuddy"]["type"], "http");
        assert!(config["mcpServers"]["vibebuddy"]["url"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        assert!(!std::fs::read_to_string(&mcp_config).unwrap().contains("sk-ant-test"), "the key stays out of files");

        // The voice command goes in as a user message, after a context block with the local time.
        let line = script.lock().unwrap().received[0].clone();
        assert_eq!(line["type"], "user");
        let content = line["message"]["content"].as_array().unwrap();
        assert!(content[0]["text"].as_str().unwrap().starts_with("<context>The user's local time: "));
        assert_eq!(content[1], json!({"type": "text", "text": "hello"}));
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(STATE_FILE)).unwrap()).unwrap();
        assert_eq!(saved["session_id"], session.as_str());
        assert_eq!(brain.session_id().as_deref(), Some(session.as_str()));

        // One process for good: the next command goes to the same one.
        say(&brain, "sk-ant-test", "again").await.unwrap();
        assert_eq!(launches(&script).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_spoken_reply_is_handed_over_when_speak_is_called_not_when_the_turn_ends() {
        let dir = temp_dir("claude-stream");
        let script = script();
        let hold = Arc::new(tokio::sync::Notify::new());
        script.lock().unwrap().hold = Some(hold.clone());
        script.lock().unwrap().turns.push_back(vec![
            Step::Init(API_KEY_SOURCE),
            Step::Call("speak", json!({"text": " On it. "})),
            Step::Hold,
            Step::Call("speak", json!({"text": "Done."})),
            Step::Result(false, ""),
        ]);
        let brain = Arc::new(brain(&dir, &script, Arc::default()));
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let turn = tokio::spawn({
            let brain = brain.clone();
            async move {
                brain.say_streaming("key", "instructions", &robot(), "hi", &mut |reply| sender.send(reply.to_owned()).unwrap()).await
            }
        });
        assert_eq!(receiver.recv().await.as_deref(), Some("On it."));
        assert!(!turn.is_finished(), "the turn is still running");
        hold.notify_one();
        assert_eq!(turn.await.unwrap().unwrap(), brain.session_id().unwrap());
        assert_eq!(receiver.recv().await.as_deref(), Some("Done."), "said just before the result, still heard");
        let answers = script.lock().unwrap().answers.clone();
        assert_eq!(answers[0].0, 200);
        assert_eq!(answers[0].1["result"], json!({"content": [{"type": "text", "text": "spoken"}], "isError": false}));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn list_activities_reads_the_tracker_and_other_tools_are_refused() {
        let dir = temp_dir("claude-tools");
        let mut tracker = ActivityTracker::default();
        let id = ActivityId { session_id: "s".to_owned(), key: "s:t".to_owned() };
        tracker.observe(&id, "CX:REVIEW", ActivityStatus::Working);
        let script = script();
        script.lock().unwrap().turns.push_back(vec![
            Step::Init(API_KEY_SOURCE),
            Step::Call("list_activities", json!({})),
            Step::Call("search_memories", json!({"query": "cat"})),
            Step::Call("run_shell", json!({"command": "ls"})),
            Step::Call("speak", json!({"text": ""})),
            Step::Result(false, ""),
        ]);
        let brain = brain(&dir, &script, Arc::new(Mutex::new(tracker)));
        assert_eq!(say(&brain, "key", "what's up?").await.unwrap(), Vec::<String>::new());
        let answers = script.lock().unwrap().answers.clone();
        let text = |index: usize| answers[index].1["result"]["content"][0]["text"].as_str().unwrap().to_owned();
        let listed: Value = serde_json::from_str(&text(0)).unwrap();
        assert_eq!(listed["activities"][0]["title"], "CX:REVIEW");
        assert!(listed["recently_ended"].is_array() && listed["today"].is_object());
        assert_eq!(answers[1].1["result"]["isError"], true, "Codex memories aren't Claude's");
        assert_eq!(answers[2].1["result"]["isError"], true);
        assert_eq!(text(2), "unknown tool run_shell");
        assert_eq!(answers[3].1["result"]["isError"], true, "an empty reply isn't spoken");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_mcp_server_answers_only_with_its_token_and_lists_only_our_tools() {
        let dir = temp_dir("claude-mcp");
        let brain = brain(&dir, &script(), Arc::default());
        say(&brain, "key", "hi").await.unwrap();
        let config = dir.join("resident-claude/mcp.json");

        let (status, _) = post_to_mcp(&config, Some("wrong"), json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await;
        assert_eq!(status, 401);
        let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "2"}}});
        let (status, answer) = post_to_mcp(&config, None, initialize).await;
        assert_eq!(status, 200);
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(answer["result"]["capabilities"], json!({"tools": {}}));
        let (status, _) = post_to_mcp(&config, None, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
        assert_eq!(status, 202);
        let (_, listed) = post_to_mcp(&config, None, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["speak", "list_activities"]);
        assert!(listed["result"]["tools"][0]["inputSchema"]["properties"]["text"].is_object());
        let (_, unknown) = post_to_mcp(&config, None, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})).await;
        assert_eq!(unknown["error"]["code"], -32601);
        // Outside a turn a spoken reply has nowhere to go; it is answered but dropped.
        let speak = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "speak", "arguments": {"text": "Hi."}}});
        let (_, spoken) = post_to_mcp(&config, None, speak).await;
        assert_eq!(spoken["result"]["isError"], false);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_later_start_resumes_the_session_and_one_whose_transcript_is_gone_is_replaced() {
        let dir = temp_dir("claude-resume");
        let first = script();
        say(&brain(&dir, &first, Arc::default()), "key", "hi").await.unwrap();
        let session = argument(&launches(&first)[0].0, "--session-id").unwrap();

        // Claude Code wrote the transcript under the working directory's project folder.
        let project = dir.join("resident-claude/projects/-state-resident");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{session}.jsonl")), "").unwrap();
        let second = script();
        let restarted = brain(&dir, &second, Arc::default());
        assert_eq!(restarted.session_id().as_deref(), Some(session.as_str()), "read back from the state file");
        say(&restarted, "key", "hi").await.unwrap();
        let args = &launches(&second)[0].0;
        assert_eq!(argument(args, "--resume").as_deref(), Some(session.as_str()));
        assert!(!args.iter().any(|arg| arg == "--session-id"));

        std::fs::remove_dir_all(dir.join("resident-claude/projects")).unwrap();
        let third = script();
        let replaced = brain(&dir, &third, Arc::default());
        say(&replaced, "key", "hi").await.unwrap();
        let new = argument(&launches(&third)[0].0, "--session-id").unwrap();
        assert_ne!(new, session);
        assert_eq!(replaced.session_id().as_deref(), Some(new.as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_new_character_resumes_the_same_session_with_its_instructions_and_a_note() {
        let dir = temp_dir("claude-character");
        let script = script();
        let brain = brain(&dir, &script, Arc::default());
        async fn say_as(brain: &ClaudeBrain, character: &str, text: &str) {
            let persona = Persona::of(&crate::resident::tests::wearing(character));
            let instructions = crate::resident::instructions(&persona, None, crate::resident::Brain::Claude);
            brain.say_streaming("key", &instructions, &persona, text, &mut |_| {}).await.unwrap();
        }
        let texts = |script: &Shared, index: usize| -> Vec<String> {
            let message = script.lock().unwrap().received[index].clone();
            let content = message["message"]["content"].as_array().unwrap().clone();
            content.iter().skip(1).map(|block| block["text"].as_str().unwrap().to_owned()).collect()
        };

        say_as(&brain, "ada", "hi").await;
        let session = argument(&launches(&script)[0].0, "--session-id").unwrap();
        let project = dir.join("resident-claude/projects/-state-resident");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{session}.jsonl")), "").unwrap();
        say_as(&brain, "ada", "again").await;
        assert_eq!(launches(&script).len(), 1, "the same Character keeps the process");
        assert_eq!(texts(&script, 0), ["hi"], "the first command has nothing to switch from");

        say_as(&brain, "wanwanxiaohe", "你好").await;
        let launched = launches(&script);
        assert_eq!(launched.len(), 2, "--system-prompt is per process");
        assert_eq!(argument(&launched[1].0, "--resume").as_deref(), Some(session.as_str()), "the same session, never a fork");
        let prompt = argument(&launched[1].0, "--system-prompt").unwrap();
        assert!(prompt.contains("Right now you are Xiaohe (小何)") && prompt.contains("always reply in Chinese"), "{prompt}");
        let switched = texts(&script, 2);
        assert!(switched[0].starts_with(CONTEXT_PREFIX) && switched[0].contains("from now on you are Xiaohe (小何)"), "{switched:?}");
        assert_eq!(switched[1], "你好");

        say_as(&brain, "wanwanxiaohe", "还在吗").await;
        assert_eq!(launches(&script).len(), 2);
        assert_eq!(texts(&script, 3), ["还在吗"], "the note goes once");
        // Whom the session spoke as is kept across a restart.
        assert!(std::fs::read_to_string(dir.join(STATE_FILE)).unwrap().contains("\"wanwanxiaohe\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_credential_other_than_the_api_key_stops_the_process() {
        let dir = temp_dir("claude-oauth");
        let script = script();
        script.lock().unwrap().turns.push_back(vec![Step::Init("none"), Step::Result(false, "")]);
        let brain = brain(&dir, &script, Arc::default());
        let error = say(&brain, "key", "hi").await.unwrap_err();
        assert!(matches!(&error, ResidentError::Rpc(message) if message.contains("not the Anthropic API key")), "{error}");
        say(&brain, "key", "hi").await.unwrap();
        assert_eq!(launches(&script).len(), 2, "the refused process was dropped");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_error_result_fails_the_turn_and_a_dead_process_is_started_again() {
        let dir = temp_dir("claude-errors");
        let script = script();
        script.lock().unwrap().turns.extend([
            vec![Step::Init(API_KEY_SOURCE), Step::Result(true, "Invalid API key · Fix external API key")],
            vec![Step::Exit],
        ]);
        let brain = brain(&dir, &script, Arc::default());
        let error = say(&brain, "key", "one").await.unwrap_err();
        assert_eq!(error, ResidentError::TurnEnded("Invalid API key · Fix external API key".to_owned()));
        assert!(matches!(say(&brain, "key", "two").await, Err(ResidentError::Rpc(_))));
        assert_eq!(launches(&script).len(), 1, "an error result leaves the process running");
        say(&brain, "key", "three").await.unwrap();
        assert_eq!(launches(&script).len(), 2, "a process that died is started again");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_new_key_starts_a_new_process_and_stop_drops_it() {
        let dir = temp_dir("claude-key");
        let script = script();
        let brain = brain(&dir, &script, Arc::default());
        say(&brain, "old", "hi").await.unwrap();
        say(&brain, "new", "hi").await.unwrap();
        let keys: Vec<String> = launches(&script)
            .iter()
            .map(|(_, env)| env.iter().find(|(name, _)| name == "ANTHROPIC_API_KEY").unwrap().1.clone())
            .collect();
        assert_eq!(keys, ["old", "new"]);
        brain.stop().await;
        say(&brain, "new", "hi").await.unwrap();
        assert_eq!(launches(&script).len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_turn_that_takes_too_long_stops_the_process() {
        let dir = temp_dir("claude-timeout");
        let script = script();
        script.lock().unwrap().hold = Some(Arc::new(tokio::sync::Notify::new()));
        script.lock().unwrap().turns.push_back(vec![Step::Init(API_KEY_SOURCE), Step::Hold]);
        let brain = brain(&dir, &script, Arc::default());
        assert_eq!(say(&brain, "key", "hi").await.unwrap_err(), ResidentError::Timeout);
        say(&brain, "key", "hi").await.unwrap();
        assert_eq!(launches(&script).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_history_is_read_from_the_transcript_in_time_order() {
        let line = |time: &str, entry: Value| {
            let mut entry = entry;
            entry["timestamp"] = json!(time);
            entry.to_string()
        };
        let speak = |text: &str| json!({"type": "tool_use", "name": "mcp__vibebuddy__speak", "input": {"text": text}});
        let transcript = [
            // Written out of order, as Claude Code does.
            line("2026-10-14T07:00:02Z", json!({"type": "assistant", "message": {"content": [speak("One task."), speak("It's fine.")]}})),
            line("2026-10-14T07:00:01Z", json!({"type": "user", "message": {"content": [
                {"type": "text", "text": "<context>The user's local time: Tuesday</context>"},
                {"type": "text", "text": "what's up?"},
            ]}})),
            json!({"type": "queue-operation", "operation": "enqueue", "content": "what's up?"}).to_string(),
            line("2026-10-14T07:00:03Z", json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "x", "content": "spoken"}]}})),
            line("2026-10-14T07:01:00Z", json!({"type": "user", "message": {"content": "and now?"}})),
            line("2026-10-14T07:01:01Z", json!({"type": "assistant", "message": {"content": [
                {"type": "tool_use", "name": "mcp__vibebuddy__list_activities", "input": {}},
                {"type": "text", "text": "thinking aloud"},
            ]}})),
            line("2026-10-14T07:01:02Z", json!({"type": "assistant", "message": {"content": [speak("Still one.")]}})),
            line("2026-10-14T07:01:03Z", json!({"type": "user", "isMeta": true, "message": {"content": "meta"}})),
            line("2026-10-14T07:01:04Z", json!({"type": "user", "isSidechain": true, "message": {"content": "sub"}})),
            "not json".to_owned(),
        ]
        .join("\n");
        assert_eq!(
            exchanges_of(&transcript),
            [
                Exchange { said: "what's up?".to_owned(), replies: vec!["One task.".to_owned(), "It's fine.".to_owned()] },
                Exchange { said: "and now?".to_owned(), replies: vec!["Still one.".to_owned()] },
            ]
        );

        let dir = temp_dir("claude-history");
        let brain = brain(&dir, &script(), Arc::default());
        assert_eq!(brain.history(5), [], "no session yet");
        *brain.session_id.lock().unwrap() = Some("abc".to_owned());
        let project = dir.join("resident-claude/projects/-state-resident");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), &transcript).unwrap();
        assert_eq!(brain.history(1), [Exchange { said: "and now?".to_owned(), replies: vec!["Still one.".to_owned()] }]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_child_gets_none_of_the_daemons_claude_or_anthropic_variables() {
        for name in ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_USE_BEDROCK", "CLAUDECODE", "ANTHROPIC_PROFILE", "CLAUDE_CONFIG_DIR"] {
            assert!(withheld(name), "{name}");
        }
        for name in ["HOME", "PATH", "LANG", "VIBEBUDDY_CLAUDE_BASE_URL"] {
            assert!(!withheld(name), "{name}");
        }
        let env = child_env(Path::new("/state/resident-claude"), "sk", Some("http://127.0.0.1:9".to_owned()));
        assert!(env.contains(&("ANTHROPIC_BASE_URL".to_owned(), "http://127.0.0.1:9".to_owned())));
        assert!(env.contains(&("CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_owned(), "1".to_owned())));
        assert!(!child_env(Path::new("/s"), "sk", Some(String::new())).iter().any(|(name, _)| name == "ANTHROPIC_BASE_URL"));
    }

    #[test]
    fn private_files_are_written_0600_even_over_a_wider_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("claude-private");
        let path = dir.join("secret.json");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
