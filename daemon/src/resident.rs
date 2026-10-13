//! The resident session (ADR-0017): the one long-running Codex thread every voice command goes to.
//!
//! The daemon drives the user's own `codex app-server` over stdio with the user's own login: `thread/start` once,
//! `thread/resume` on every later start, `turn/start` per voice command. The session answers through the `speak`
//! tool we define (`dynamicTools`), and reads the Activities through `list_activities`; it gets no tool that runs
//! commands or writes files, runs read-only, and every approval or question it raises is declined.
//!
//! Experimental and off unless `VIBEBUDDY_RESIDENT_SESSION=1`. The app-server starts on the first voice command
//! and again on the next one after it dies. Voice commands are taken one at a time; the ones that arrive meanwhile
//! wait their turn in arrival order.
//!
//! The resident session is never an Activity: the user's Codex hooks fire for its thread too, and
//! [`Resident::owns_hook`] is how the Codex adapter recognises and drops them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

use crate::activity::ActivityTracker;
use crate::app_server::{Client, Incoming, RpcError};

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

pub fn enabled_by_env() -> bool {
    matches!(std::env::var(ENABLE_VARIABLE).as_deref(), Ok("1" | "true"))
}

#[derive(Debug, PartialEq)]
pub enum ResidentError {
    Disabled,
    NoCodex,
    NotLoggedIn,
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

/// What the resident session said in answer to one voice command.
#[derive(Debug, Serialize)]
pub struct Answer {
    pub thread_id: String,
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
    _child: Option<tokio::process::Child>,
}

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    thread_id: Option<String>,
    /// The Codex version the thread was last resumed with, for diagnosing a protocol change.
    codex_version: Option<String>,
}

pub struct Resident {
    enabled: bool,
    /// The working directory the resident thread runs in; the daemon owns it.
    dir: PathBuf,
    /// The same directory with symlinks resolved, as Codex may report it in hooks.
    canonical_dir: PathBuf,
    state_file: PathBuf,
    launcher: Launcher,
    activities: Arc<Mutex<ActivityTracker>>,
    /// Held for a whole turn: one voice command at a time, the rest wait here in order.
    session: Mutex<Option<Session>>,
    /// Read synchronously by the hook filter.
    thread_id: std::sync::Mutex<Option<String>>,
}

impl Resident {
    /// `state_dir` is the daemon's state directory: the thread id is kept in `resident.json` there, and the
    /// session works in `resident/`.
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
            launcher,
            activities,
            session: Mutex::new(None),
            thread_id: std::sync::Mutex::new(saved.thread_id),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn thread_id(&self) -> Option<String> {
        self.thread_id.lock().expect("thread id lock").clone()
    }

    /// Whether a Codex hook event comes from the resident session: by its thread, or by its working directory,
    /// which also covers the user opening the thread in the Codex App and typing there.
    pub fn owns_hook(&self, session_id: &str, thread_id: Option<&str>, cwd: Option<&str>) -> bool {
        if let Some(resident) = self.thread_id.lock().expect("thread id lock").as_deref()
            && (session_id == resident || thread_id == Some(resident))
        {
            return true;
        }
        cwd.map(Path::new)
            .is_some_and(|cwd| cwd.starts_with(&self.dir) || cwd.starts_with(&self.canonical_dir))
    }

    /// Hands one voice command to the resident session and waits for its spoken replies.
    pub async fn say(&self, text: &str) -> Result<Answer, ResidentError> {
        let mut replies = Vec::new();
        let thread_id = self.say_streaming(text, &mut |reply| replies.push(reply.to_owned())).await?;
        Ok(Answer { thread_id, replies })
    }

    /// Hands one voice command to the resident session; each spoken reply goes to `on_reply` the moment the
    /// session calls `speak`, not when the turn completes, which in testing was 4 to 6 seconds later. Returns the
    /// thread id once the turn has completed.
    pub async fn say_streaming(&self, text: &str, on_reply: &mut (dyn FnMut(&str) + Send)) -> Result<String, ResidentError> {
        if !self.enabled {
            return Err(ResidentError::Disabled);
        }
        let mut slot = self.session.lock().await;
        if slot.as_ref().is_some_and(|session| session.client.is_closed()) {
            warn!("the resident app-server exited; starting it again");
            *slot = None;
        }
        if slot.is_none() {
            *slot = Some(self.start().await?);
        }
        let session = slot.as_mut().expect("session was just started");
        let result = self.run_turn(session, text, on_reply).await;
        if session.client.is_closed() {
            *slot = None;
        }
        result
    }

    async fn start(&self) -> Result<Session, ResidentError> {
        let (pipes, version) = match &self.launcher {
            Launcher::Codex => spawn_codex().await?,
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
            return Err(ResidentError::NotLoggedIn);
        }
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| ResidentError::Rpc(format!("cannot create {}: {error}", self.dir.display())))?;
        let thread_id = match self.thread_id() {
            Some(saved) => match self.resume(&client, &saved).await {
                Err(ResidentError::Rpc(message)) if !client.is_closed() => {
                    let thread = self.start_thread(&client).await?;
                    warn!(old_thread = %saved, new_thread = %thread, %message, "the resident thread could not be resumed; started a new one");
                    thread
                }
                other => other?,
            },
            None => self.start_thread(&client).await?,
        };
        *self.thread_id.lock().expect("thread id lock") = Some(thread_id.clone());
        let saved = Saved { thread_id: Some(thread_id.clone()), codex_version: version };
        if let Err(error) = save(&self.state_file, &saved) {
            warn!(%error, "failed to save the resident thread id");
        }
        info!(thread = %thread_id, "resident session ready");
        Ok(Session { client, incoming, thread_id, _child: pipes.child })
    }

    async fn start_thread(&self, client: &Client) -> Result<String, ResidentError> {
        let mut params = self.thread_params();
        params["dynamicTools"] = tools();
        let started = request(client, "thread/start", params).await?;
        thread_id_of(&started)
    }

    async fn resume(&self, client: &Client, thread_id: &str) -> Result<String, ResidentError> {
        // Dynamic tools live with the thread and survive a resume; the instructions are sent again so a change
        // (a new Character, later) takes effect.
        let mut params = self.thread_params();
        params["threadId"] = json!(thread_id);
        let resumed = request(client, "thread/resume", params).await?;
        thread_id_of(&resumed)
    }

    fn thread_params(&self) -> Value {
        json!({
            "cwd": self.dir,
            "sandbox": "read-only",
            "approvalPolicy": "on-request",
            "developerInstructions": instructions(None),
        })
    }

    async fn run_turn(
        &self,
        session: &mut Session,
        text: &str,
        on_reply: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String, ResidentError> {
        // Whatever an earlier, timed-out turn left behind still gets its answer, but no longer counts.
        while let Ok(message) = session.incoming.try_recv() {
            self.handle(&session.client, message, &mut |_| {}).await;
        }
        let started = request(
            &session.client,
            "turn/start",
            json!({"threadId": session.thread_id, "input": [{"type": "text", "text": text}]}),
        )
        .await?;
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
        let (success, text) = match tool {
            "speak" => {
                let text = params.pointer("/arguments/text").and_then(Value::as_str).unwrap_or_default().trim();
                if text.is_empty() {
                    (false, "text is empty".to_owned())
                } else {
                    info!(chars = text.chars().count(), "the resident session spoke");
                    on_reply(text);
                    (true, "spoken".to_owned())
                }
            }
            "list_activities" => {
                let activities = self.activities.lock().await.summaries();
                (true, json!({"activities": activities}).to_string())
            }
            other => (false, format!("unknown tool {other}")),
        };
        json!({"success": success, "contentItems": [{"type": "inputText", "text": text}]})
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
fn tools() -> Value {
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
            "description": "List what the user's coding Agents are doing right now, newest first: agent, task card title, \
                project, state (working or needs_input), where it runs, and seconds in that state.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        },
    ])
}

/// The developer instructions. `persona` is the seam for the Character the user picked (CONTEXT.md, Persona);
/// until Characters carry one, the buddy speaks plainly.
fn instructions(persona: Option<&str>) -> String {
    let persona = persona.unwrap_or("You are friendly, calm and brief.");
    format!(
        "You are the buddy of VibeBuddy: a small desk box that watches the user's coding Agents (Codex, Claude Code, \
         OpenCode, GitHub Copilot CLI, Pi, GitHub Actions) and speaks up for them. {persona}\n\
         Every user message is one voice command, transcribed from speech. Answer only by calling the `speak` tool, \
         with one or two short sentences meant to be heard, not read: no Markdown, lists, code, paths or URLs. \
         Reply in the language the user spoke. Do not write any other answer.\n\
         To know what the Agents are doing, call `list_activities`; never guess. Titles there are short upper-case \
         task-card names; say them naturally.\n\
         You do no work yourself: run no commands, read or edit no files, browse nothing. If the user asks for work \
         to be done, say briefly that you can't start it yet."
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

/// Starts `codex app-server` and checks its version against the one this was tested with.
async fn spawn_codex() -> Result<(Pipes, Option<String>), ResidentError> {
    let program = find_codex().ok_or(ResidentError::NoCodex)?;
    let version = tokio::process::Command::new(&program)
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
    let servers = enabled_mcp_servers(&program, &base).await;
    let mut child = tokio::process::Command::new(&program)
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

/// Features that give a Codex thread tools of its own: the resident session gets none of them, only ours. Plugins
/// and apps (connectors) bring tools and MCP servers; the rest are built-in tools. Set as `-c features.<name>=false`
/// rather than `--disable`, which refuses a name another Codex version doesn't know. The user's hooks and
/// AGENTS.md stay: the hooks are filtered, and AGENTS.md only shapes how it talks.
const FEATURES_OFF: [&str; 15] = [
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
];

/// The config overrides that strip a Codex thread of its own tools, before MCP servers.
fn tool_overrides() -> Vec<String> {
    let mut args: Vec<String> =
        FEATURES_OFF.iter().flat_map(|feature| ["-c".to_owned(), format!("features.{feature}=false")]).collect();
    args.extend(["-c".to_owned(), "web_search=\"disabled\"".to_owned()]);
    args
}

/// `codex app-server` with the tool overrides and every MCP server the user configured switched off. An override
/// can't empty `mcp_servers` (it merges), so each server is turned off by name.
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
async fn enabled_mcp_servers(program: &Path, base: &[String]) -> Vec<String> {
    let output = tokio::process::Command::new(program)
        .args(base)
        .args(["mcp", "list", "--json"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    let listed = output.ok().and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
    let Some(servers) = listed.as_ref().and_then(Value::as_array) else {
        warn!("could not list the user's MCP servers; the resident session may see their tools");
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
                let result = match method.as_str() {
                    "initialized" => continue,
                    "initialize" => json!({"userAgent": "fake"}),
                    "account/read" => {
                        let logged_in = script.lock().unwrap().logged_in;
                        let account = if logged_in { json!({"type": "chatgpt"}) } else { Value::Null };
                        json!({"account": account, "requiresOpenaiAuth": true})
                    }
                    "thread/start" => json!({"thread": {"id": "new-thread"}}),
                    "thread/resume" if lose_thread => {
                        let error = json!({"id": id, "error": {"code": -32600, "message": "no rollout found"}});
                        out.write_all(format!("{error}\n").as_bytes()).await.unwrap();
                        continue;
                    }
                    "thread/resume" => json!({"thread": {"id": message["params"]["threadId"]}}),
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

    pub(crate) fn resident(dir: &Path, script: &Shared, activities: Arc<Mutex<ActivityTracker>>) -> Resident {
        let script = script.clone();
        Resident::new(true, dir, Launcher::Fake(Box::new(move || fake_server(script.clone()))), activities)
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

        let answer = resident.say("hello").await.unwrap();
        assert_eq!(answer.thread_id, "new-thread");
        assert_eq!(methods(&script), ["initialize", "initialized", "account/read", "thread/start", "turn/start"]);
        let received = script.lock().unwrap().received.clone();
        assert_eq!(received[0]["params"]["capabilities"]["experimentalApi"], true);
        assert_eq!(received[0]["params"]["clientInfo"]["name"], "vibebuddy");
        let start = &received[3]["params"];
        assert_eq!(start["cwd"], json!(dir.join("resident")));
        assert_eq!(start["sandbox"], "read-only");
        assert_eq!(start["approvalPolicy"], "on-request");
        assert!(start["developerInstructions"].as_str().unwrap().contains("speak"));
        let tools: Vec<&str> =
            start["dynamicTools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(tools, ["speak", "list_activities"]);
        assert_eq!(received[4]["params"], json!({"threadId": "new-thread", "input": [{"type": "text", "text": "hello"}]}));
        assert!(dir.join("resident").is_dir());
        assert_eq!(load(&dir.join("resident.json")).thread_id.as_deref(), Some("new-thread"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_later_start_resumes_the_saved_thread_and_a_lost_one_is_replaced() {
        let dir = temp_dir("resume");
        let saved = Saved { thread_id: Some("old-thread".to_owned()), codex_version: None };
        save(&dir.join("resident.json"), &saved).unwrap();
        let script = logged_in();
        let answer = resident(&dir, &script, Arc::default()).say("hi").await.unwrap();
        assert_eq!(answer.thread_id, "old-thread");
        assert_eq!(methods(&script), ["initialize", "initialized", "account/read", "thread/resume", "turn/start"]);
        let resume = script.lock().unwrap().received[3]["params"].clone();
        assert_eq!(resume["threadId"], "old-thread");
        assert_eq!(resume["sandbox"], "read-only");
        assert_eq!(resume["approvalPolicy"], "on-request");

        let script = logged_in();
        script.lock().unwrap().lose_thread = true;
        let answer = resident(&dir, &script, Arc::default()).say("hi").await.unwrap();
        assert_eq!(answer.thread_id, "new-thread");
        assert_eq!(methods(&script)[3..5], ["thread/resume", "thread/start"]);
        assert_eq!(load(&dir.join("resident.json")).thread_id.as_deref(), Some("new-thread"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn without_a_login_nothing_starts_and_no_login_flow_is_begun() {
        let dir = temp_dir("login");
        let script = Arc::new(std::sync::Mutex::new(Script::default()));
        let error = resident(&dir, &script, Arc::default()).say("hi").await.unwrap_err();
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
        let script = logged_in();
        script.lock().unwrap().turn_requests = vec![
            tool_call("list_activities", json!({})),
            tool_call("speak", json!({"text": " One task is running. "})),
            tool_call("speak", json!({"text": ""})),
            tool_call("run_shell", json!({"command": "ls"})),
        ];
        let answer = resident(&dir, &script, Arc::new(Mutex::new(tracker))).say("what's up?").await.unwrap();
        assert_eq!(answer.replies, ["One task is running."]);

        let answers = script.lock().unwrap().answers.clone();
        let text = |index: usize| answers[index]["result"]["contentItems"][0]["text"].as_str().unwrap().to_owned();
        assert_eq!(answers[0]["id"], "srv-1-0");
        assert_eq!(answers[0]["result"]["success"], true);
        let listed: Value = serde_json::from_str(&text(0)).unwrap();
        assert_eq!(listed["activities"][0]["title"], "CX:REVIEW");
        assert_eq!(listed["activities"][0]["state"], "working");
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
            async move { resident.say_streaming("hi", &mut |reply| sender.send(reply.to_owned()).unwrap()).await }
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
        resident(&dir, &script, Arc::default()).say("delete everything").await.unwrap();
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
        assert!(matches!(resident.say("one").await, Err(ResidentError::Rpc(_))));
        let answer = resident.say("two").await.unwrap();
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
        let (first, second) = tokio::join!(resident.say("one"), resident.say("two"));
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
        assert_eq!(resident.say("hi").await.unwrap_err(), ResidentError::Disabled);
        assert_eq!(script.lock().unwrap().starts, 0);
        let _ = std::fs::remove_dir_all(dir);
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
            "web_search=\"disabled\"",
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
    fn hooks_from_the_resident_thread_or_its_directory_are_recognised() {
        let dir = temp_dir("hooks");
        let saved = Saved { thread_id: Some("resident-thread".to_owned()), codex_version: None };
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
