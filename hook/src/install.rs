//! `vibebuddy-hook install` / `uninstall`: where there is no app (Linux), the hook registers itself in the
//! user-level config of Claude Code and Codex, and writes or deletes the file it owns for OpenCode, Copilot and Pi
//! (`cli_agents`). Same rules as the app's `HookConfig.swift`: only VibeBuddy's own
//! entries are added or removed, a `.bak` is kept, and the new file is swapped in whole.
//!
//! An install that changes nothing leaves the file untouched: Codex keys its trust to a hash of each hook, so
//! rewriting an unchanged config could still cost the user a trip to `/hooks`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::cli_agents;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn argument(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }

    fn events(self) -> &'static [&'static str] {
        match self {
            Agent::Claude => &[
                "UserPromptSubmit",
                "PermissionRequest",
                "PostToolUse",
                "Stop",
                "StopFailure",
                "SubagentStart",
                "SubagentStop",
                "SessionEnd",
            ],
            Agent::Codex => &[
                "UserPromptSubmit",
                "PermissionRequest",
                "PostToolUse",
                "Stop",
                "Interrupt",
                "SessionEnd",
            ],
        }
    }

    /// The agent's user directory; it exists once the agent has run on this machine.
    fn dir(self) -> &'static str {
        match self {
            Agent::Claude => ".claude",
            Agent::Codex => ".codex",
        }
    }

    fn config_file(self) -> &'static str {
        match self {
            Agent::Claude => "settings.json",
            Agent::Codex => "hooks.json",
        }
    }
}

/// Whether a command is ours; the old names count too, and get replaced.
fn is_ours(command: &str) -> bool {
    [
        "vibebuddy-hook",
        "beacon-hook",
        "codex-hook.py",
        "claude-hook.py",
    ]
    .iter()
    .any(|name| command.contains(name))
}

/// What a hook entry runs: its `command`, and the `commandWindows` that Codex on Windows runs instead, if any.
#[derive(PartialEq)]
struct Call {
    command: String,
    windows: Option<String>,
}

impl Call {
    fn of(entry: &Value) -> Option<Call> {
        let command = entry.get("command")?.as_str()?.to_owned();
        let windows = entry.get("commandWindows").and_then(Value::as_str).map(str::to_owned);
        Some(Call { command, windows })
    }

    fn entry(&self) -> Value {
        let mut entry = json!({ "type": "command", "command": self.command, "timeout": 2 });
        if let Some(windows) = &self.windows {
            entry["commandWindows"] = json!(windows);
        }
        entry
    }

    /// One line for the diff shown before writing.
    fn shown(&self) -> String {
        match &self.windows {
            Some(windows) => format!("{} (Windows: {windows})", self.command),
            None => self.command.clone(),
        }
    }
}

/// Claude Code runs hooks in Git Bash on Windows, as everywhere else, so one command does.
fn wanted(binary: &str, agent: Agent) -> Call {
    if cfg!(windows) && agent == Agent::Codex {
        return windows_codex_call(binary);
    }
    Call { command: format!("\"{binary}\" {}", agent.argument()), windows: None }
}

/// Codex on Windows runs `commandWindows` in PowerShell, where a quoted path is a string, not a call: `&` calls it.
/// Codex in a WSL workspace is the Linux build reading the same `hooks.json` through `/mnt/c`, and runs `command`
/// in the user's POSIX shell. There `&` is a syntax error, which exits with 2, and Codex reads 2 as "block": so
/// `command` holds the path as WSL sees it.
fn windows_codex_call(binary: &str) -> Call {
    let call = |path: &str| format!("\"{path}\" {}", Agent::Codex.argument());
    Call { command: call(&wsl_path(binary)), windows: Some(format!("& {}", call(binary))) }
}

/// `C:\Users\me\x.exe` at WSL's default mount point: `/mnt/c/Users/me/x.exe`. Any other path stays as it is; WSL
/// then doesn't find it, and exit code 127 is a hook failure that Codex doesn't treat as a block.
fn wsl_path(path: &str) -> String {
    let mut chars = path.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(drive), Some(':'), Some('\\')) if drive.is_ascii_alphabetic() => {
            format!("/mnt/{}/{}", drive.to_ascii_lowercase(), path[3..].replace('\\', "/"))
        }
        _ => path.to_owned(),
    }
}

/// Adds our hook to every event that lacks it and drops our stale entries; everything else keeps its place.
fn install(root: Map<String, Value>, agent: Agent, binary: &str) -> Map<String, Value> {
    let wanted = wanted(binary, agent);
    let mut root = remove_where(root, |call| is_ours(&call.command) && *call != wanted);
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let Value::Object(hooks) = hooks else {
        return root;
    };
    for event in agent.events() {
        let groups = hooks.entry(*event).or_insert_with(|| json!([]));
        if !groups.is_array() {
            *groups = json!([]);
        }
        let Value::Array(groups) = groups else {
            continue;
        };
        if !groups
            .iter()
            .any(|group| calls(group).any(|call| call == wanted))
        {
            groups.push(json!({ "hooks": [wanted.entry()] }));
        }
    }
    root
}

fn uninstall(root: Map<String, Value>) -> Map<String, Value> {
    remove_where(root, |call| is_ours(&call.command))
}

/// Removes matching hook commands, then any group, event and `hooks` object left empty by that.
/// Shapes we don't recognise are someone else's and are left alone.
fn remove_where(
    mut root: Map<String, Value>,
    matches: impl Fn(&Call) -> bool,
) -> Map<String, Value> {
    let Some(Value::Object(hooks)) = root.get_mut("hooks") else {
        return root;
    };
    hooks.retain(|_, groups| {
        let Value::Array(groups) = groups else {
            return true;
        };
        groups.retain_mut(|group| {
            let Some(Value::Array(entries)) = group.get_mut("hooks") else {
                return true;
            };
            entries.retain(|entry| !Call::of(entry).is_some_and(|call| matches(&call)));
            !entries.is_empty()
        });
        !groups.is_empty()
    });
    if hooks.is_empty() {
        root.remove("hooks");
    }
    root
}

fn calls(group: &Value) -> impl Iterator<Item = Call> {
    group.get("hooks").and_then(Value::as_array).into_iter().flatten().filter_map(Call::of)
}

/// One agent VibeBuddy connects to: entries merged into a config the agent shares with the user, or a whole file
/// of VibeBuddy's own. In the Mac app's order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Merged(Agent),
    Owned(cli_agents::Agent),
}

impl Target {
    const ALL: [Target; 5] = [
        Target::Merged(Agent::Codex),
        Target::Merged(Agent::Claude),
        Target::Owned(cli_agents::Agent::OpenCode),
        Target::Owned(cli_agents::Agent::Copilot),
        Target::Owned(cli_agents::Agent::Pi),
    ];

    fn from_argument(argument: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.argument() == argument)
    }

    fn argument(self) -> &'static str {
        match self {
            Target::Merged(agent) => agent.argument(),
            Target::Owned(agent) => agent.argument(),
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Target::Merged(agent) => agent.display_name(),
            Target::Owned(agent) => agent.display_name(),
        }
    }

    /// Exists once the agent has run on this machine.
    fn dir(self, home: &Path) -> PathBuf {
        match self {
            Target::Merged(agent) => home.join(agent.dir()),
            Target::Owned(agent) => agent.dir(home),
        }
    }

    fn path(self, home: &Path) -> PathBuf {
        match self {
            Target::Merged(agent) => home.join(agent.dir()).join(agent.config_file()),
            Target::Owned(agent) => agent.file(home),
        }
    }
}

/// What connecting or removing would do to one agent's config, worked out before anything is written.
struct Change {
    target: Target,
    path: PathBuf,
    after: Contents,
    /// The diff shown before writing, as on the Mac: `+ Event: command` / `- Event: command` for a shared config,
    /// the whole file line by line for one of our own. Empty when nothing would change.
    lines: Vec<String>,
}

enum Contents {
    Merged(Map<String, Value>),
    /// None deletes the file.
    Owned(Option<String>),
}

fn plan(target: Target, home: &Path, binary: &str, installing: bool) -> Result<Change, String> {
    let path = target.path(home);
    let (after, lines) = match target {
        Target::Merged(agent) => {
            let before = read(&path)?;
            let after = if installing { install(before.clone(), agent, binary) } else { uninstall(before.clone()) };
            let lines = describe_change(&before, &after);
            (Contents::Merged(after), lines)
        }
        Target::Owned(agent) => {
            let before = std::fs::read_to_string(&path).ok();
            let after = installing.then(|| agent.file_contents(binary));
            let lines = describe_owned_file(before.as_deref(), after.as_deref());
            (Contents::Owned(after), lines)
        }
    };
    Ok(Change { target, path, after, lines })
}

fn apply(change: &Change, home: &Path) -> Result<(), String> {
    let path = &change.path;
    let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
    match &change.after {
        Contents::Merged(root) => write(path, root),
        Contents::Owned(Some(text)) => {
            std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(fail)?;
            std::fs::write(path, text).map_err(fail)
        }
        Contents::Owned(None) => std::fs::remove_file(path).map_err(fail),
    }
}

/// Every event carries our command (a shared config), or our file runs this binary (one of our own); the same test
/// as the Mac's `HookConfig.isInstalled`.
fn is_installed(target: Target, home: &Path, binary: &str) -> bool {
    let path = target.path(home);
    match target {
        Target::Merged(agent) => {
            let wanted = wanted(binary, agent);
            let Ok(root) = read(&path) else { return false };
            agent.events().iter().all(|event| {
                root.get("hooks")
                    .and_then(|hooks| hooks.get(*event))
                    .and_then(Value::as_array)
                    .is_some_and(|groups| groups.iter().any(|group| calls(group).any(|call| call == wanted)))
            })
        }
        // Our files hold the path as a JSON string, so a Windows path's backslashes are doubled there.
        Target::Owned(_) => {
            let quoted = Value::String(binary.to_owned()).to_string();
            std::fs::read_to_string(&path).is_ok_and(|text| text.contains(&quoted))
        }
    }
}

fn describe_change(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let events = |root: &Map<String, Value>| -> std::collections::BTreeMap<String, Vec<String>> {
        root.get("hooks")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(event, groups)| {
                let commands = groups.as_array().into_iter().flatten().flat_map(calls).map(|call| call.shown()).collect();
                (event.clone(), commands)
            })
            .collect()
    };
    let (earlier, later) = (events(before), events(after));
    let names: std::collections::BTreeSet<&String> = earlier.keys().chain(later.keys()).collect();
    let mut lines = Vec::new();
    for event in names {
        let (old, new) = (earlier.get(event).cloned().unwrap_or_default(), later.get(event).cloned().unwrap_or_default());
        lines.extend(new.iter().filter(|command| !old.contains(command)).map(|command| format!("+ {event}: {command}")));
        lines.extend(old.iter().filter(|command| !new.contains(command)).map(|command| format!("- {event}: {command}")));
    }
    lines
}

fn describe_owned_file(before: Option<&str>, after: Option<&str>) -> Vec<String> {
    if before == after {
        return Vec::new();
    }
    let lines = |text: Option<&str>, sign: char| -> Vec<String> {
        text.unwrap_or_default().lines().filter(|line| !line.is_empty()).map(|line| format!("{sign} {line}")).collect()
    };
    [lines(before, '-'), lines(after, '+')].concat()
}

fn home() -> Result<PathBuf, String> {
    std::env::home_dir().ok_or_else(|| "no home directory".to_owned())
}

/// The path written into configs: this binary, wherever it was installed.
fn binary() -> Result<String, String> {
    let binary = std::env::current_exe().map_err(|error| format!("cannot find this binary: {error}"))?;
    binary.to_str().map(str::to_owned).ok_or_else(|| "this binary's path is not valid UTF-8".to_owned())
}

/// `vibebuddy-hook install|uninstall [agent]`: every agent that has run here, or just the one named.
pub fn run(installing: bool, only: Option<&str>) -> Result<(), String> {
    let (home, binary) = (home()?, binary()?);
    let targets: Vec<Target> = match only {
        Some(argument) => vec![Target::from_argument(argument).ok_or_else(|| format!("unknown agent {argument}"))?],
        None => Target::ALL.into_iter().filter(|target| target.dir(&home).is_dir()).collect(),
    };
    if targets.is_empty() {
        println!(
            "None of Claude Code, Codex, OpenCode, GitHub Copilot CLI or Pi has run here yet. Run one once, then run this again."
        );
    }
    for target in targets {
        if !target.dir(&home).is_dir() {
            return Err(format!("{} wasn't found on this computer", target.display_name()));
        }
        let change = plan(target, &home, &binary, installing)?;
        if change.lines.is_empty() {
            println!("{}: nothing to change in {}", target.display_name(), change.path.display());
            continue;
        }
        apply(&change, &home)?;
        let name = target.display_name();
        match (&change.after, target) {
            (Contents::Merged(_), Target::Merged(agent)) => println!(
                "{name}: updated {} (the old version is in {}.bak)",
                change.path.display(),
                agent.config_file()
            ),
            (Contents::Owned(Some(_)), _) => println!("{name}: wrote {}", change.path.display()),
            _ => println!("{name}: removed {}", change.path.display()),
        }
        if installing && change.target == Target::Merged(Agent::Codex) {
            println!(
                "  Codex runs changed hooks only after you trust them: open /hooks in Codex (Settings › Hooks in the Codex app) and trust the VibeBuddy entries."
            );
        }
    }
    Ok(())
}

/// `vibebuddy-hook status`: one JSON object per agent, for the Linux app's Agents tab.
pub fn status() -> Result<String, String> {
    let (home, binary) = (home()?, binary()?);
    let agents: Vec<Value> = Target::ALL
        .into_iter()
        .map(|target| {
            let path = target.path(&home);
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .map(|time| chrono::DateTime::<chrono::Local>::from(time).to_rfc3339());
            json!({
                "agent": target.argument(),
                "name": target.display_name(),
                "present": target.dir(&home).is_dir(),
                "installed": is_installed(target, &home, &binary),
                "config": path,
                "modified": modified,
                "untrusted": target == Target::Merged(Agent::Codex) && codex_untrusted(&home, &binary),
            })
        })
        .collect();
    serde_json::to_string(&agents).map_err(|error| error.to_string())
}

/// Whether Codex trusts none of our entries in `hooks.json`. Codex skips a hook it hasn't trusted without a word, so
/// this is the only way to tell "not trusted" from "not used yet". It keeps each trusted hook under
/// `[hooks.state."<hooks.json>:<event>:<group>:<handler>"]` in `config.toml`, with a `trusted_hash`. Only whether one
/// is there counts: the hash is Codex's own format. False when it can't tell.
fn codex_untrusted(home: &Path, binary: &str) -> bool {
    let codex = home.join(Agent::Codex.dir());
    let hooks_json = codex.join(Agent::Codex.config_file());
    let (Ok(root), Ok(config)) = (read(&hooks_json), std::fs::read_to_string(codex.join("config.toml"))) else {
        return false;
    };
    let path = hooks_json.display().to_string();
    // In a WSL workspace, Codex's Linux build names the same file through /mnt/c, and trusts it under that name.
    let sources = if cfg!(windows) { vec![wsl_path(&path), path] } else { vec![path] };
    none_trusted(&root, &wanted(binary, Agent::Codex), &sources, &config)
}

fn none_trusted(root: &Map<String, Value>, wanted: &Call, sources: &[String], config: &str) -> bool {
    let (bypass, trusted) = trusted_hook_keys(config);
    let mut ours = Vec::new();
    for (event, groups) in root.get("hooks").and_then(Value::as_object).into_iter().flatten() {
        for (group, entries) in groups.as_array().into_iter().flatten().enumerate() {
            for (handler, entry) in entries.get("hooks").and_then(Value::as_array).into_iter().flatten().enumerate() {
                if Call::of(entry).as_ref() == Some(wanted) {
                    ours.push(format!("{}:{group}:{handler}", snake_case(event)));
                }
            }
        }
    }
    !bypass
        && !ours.is_empty()
        && !ours.iter().any(|key| sources.iter().any(|source| trusted.contains(&format!("{source}:{key}"))))
}

/// The hook keys `config.toml` has a `trusted_hash` for, and whether `bypass_hook_trust` turns the check off.
fn trusted_hook_keys(config: &str) -> (bool, std::collections::HashSet<String>) {
    let mut trusted = std::collections::HashSet::new();
    let (mut bypass, mut section, mut in_table) = (false, None, false);
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_table = true;
            section = line.trim_start_matches('[').trim_end_matches(']').trim().strip_prefix("hooks.state.").and_then(toml_key);
        } else if let Some((name, value)) = line.split_once('=') {
            match (name.trim(), &section) {
                ("trusted_hash", Some(key)) => {
                    trusted.insert(key.clone());
                }
                ("bypass_hook_trust", _) if !in_table => bypass = value.trim() == "true",
                _ => {}
            }
        }
    }
    (bypass, trusted)
}

/// A quoted TOML key: `"…"` with `\\` and `\"` escapes, or `'…'` as it is.
fn toml_key(quoted: &str) -> Option<String> {
    if let Some(literal) = quoted.strip_prefix('\'') {
        return literal.strip_suffix('\'').map(str::to_owned);
    }
    let mut chars = quoted.strip_prefix('"')?.strip_suffix('"')?.chars();
    let mut key = String::new();
    while let Some(c) = chars.next() {
        key.push(if c == '\\' { chars.next()? } else { c });
    }
    Some(key)
}

/// `UserPromptSubmit` as Codex writes it in a hook key: `user_prompt_submit`.
fn snake_case(event: &str) -> String {
    let mut out = String::new();
    for (index, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `vibebuddy-hook plan install|uninstall <agent>`: the change, as JSON, without writing it.
pub fn preview(installing: bool, argument: &str) -> Result<String, String> {
    let target = Target::from_argument(argument).ok_or_else(|| format!("unknown agent {argument}"))?;
    let change = plan(target, &home()?, &binary()?, installing)?;
    serde_json::to_string(&json!({ "config": change.path, "lines": change.lines })).map_err(|error| error.to_string())
}

/// A missing file is an empty config; a broken one is an error, never something to overwrite.
fn read(path: &Path) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(root)) => Ok(root),
        _ => Err(format!(
            "{} is not a JSON object; fix it by hand first",
            path.display()
        )),
    }
}

fn write(path: &Path, root: &Map<String, Value>) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
    if path.exists() {
        std::fs::copy(path, path.with_extension("json.bak")).map_err(fail)?;
    }
    let mut text = serde_json::to_string_pretty(root).map_err(|error| error.to_string())?;
    text.push('\n');
    let temporary = path.with_extension("json.vibebuddy-tmp");
    std::fs::write(&temporary, text).map_err(fail)?;
    std::fs::rename(&temporary, path).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINARY: &str = "/home/me/.local/bin/vibebuddy-hook";
    const CODEX_CALL: &str = "\"/home/me/.local/bin/vibebuddy-hook\" codex";
    /// What v0.6.2 wrote on Windows: PowerShell only, a syntax error in a WSL workspace's shell.
    const POWERSHELL_ONLY: &str = "& \"/home/me/.local/bin/vibebuddy-hook\" codex";

    /// Our Codex entry. On Windows it adds the PowerShell call; BINARY isn't a drive path, so `command` keeps it.
    fn codex_entry() -> Value {
        if cfg!(windows) {
            json!({ "type": "command", "command": CODEX_CALL, "commandWindows": POWERSHELL_ONLY, "timeout": 2 })
        } else {
            json!({ "type": "command", "command": CODEX_CALL, "timeout": 2 })
        }
    }

    fn codex_shown() -> String {
        if cfg!(windows) { format!("{CODEX_CALL} (Windows: {POWERSHELL_ONLY})") } else { CODEX_CALL.to_owned() }
    }

    fn object(value: Value) -> Map<String, Value> {
        let Value::Object(map) = value else {
            panic!("not an object")
        };
        map
    }

    #[test]
    fn installs_every_event_into_an_empty_config() {
        let root = install(Map::new(), Agent::Codex, BINARY);
        let hooks = root["hooks"].as_object().expect("hooks");
        assert_eq!(hooks.len(), Agent::Codex.events().len());
        assert_eq!(
            hooks["Stop"],
            json!([{ "hooks": [codex_entry()] }])
        );
    }

    #[test]
    fn installing_twice_changes_nothing() {
        let once = install(object(json!({ "theme": "auto" })), Agent::Claude, BINARY);
        assert_eq!(install(once.clone(), Agent::Claude, BINARY), once);
    }

    #[test]
    fn other_hooks_and_settings_keep_their_place() {
        let theirs = json!({ "hooks": [{ "type": "command", "command": "other-tool" }] });
        let root = object(json!({ "theme": "auto", "hooks": { "Stop": [theirs.clone()] } }));
        let installed = install(root.clone(), Agent::Claude, BINARY);
        assert_eq!(installed["theme"], "auto");
        assert_eq!(installed["hooks"]["Stop"][0], theirs);
        assert_eq!(installed["hooks"]["Stop"].as_array().map(Vec::len), Some(2));
        assert_eq!(uninstall(installed), root);
    }

    #[test]
    fn a_stale_path_is_replaced() {
        let stale = install(Map::new(), Agent::Claude, "/old/place/vibebuddy-hook");
        let fresh = install(stale, Agent::Claude, BINARY);
        let all: Vec<String> = fresh["hooks"]
            .as_object()
            .expect("hooks")
            .values()
            .flat_map(|groups| groups.as_array().into_iter().flatten().flat_map(calls))
            .map(|call| call.command)
            .collect();
        assert!(
            all.iter()
                .all(|command| *command == format!("\"{BINARY}\" claude"))
        );
        assert_eq!(all.len(), Agent::Claude.events().len());
    }

    #[test]
    fn a_powershell_only_codex_entry_is_replaced() {
        let old = object(json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": POWERSHELL_ONLY }] }] } }));
        let fresh = install(old, Agent::Codex, BINARY);
        assert_eq!(fresh["hooks"]["Stop"], json!([{ "hooks": [codex_entry()] }]));
    }

    #[test]
    fn codex_on_windows_gets_a_wsl_command_and_a_powershell_override() {
        let call = windows_codex_call(r"C:\Users\me\AppData\Local\Programs\VibeBuddy\vibebuddy-hook.exe");
        assert_eq!(
            call.entry(),
            json!({
                "type": "command",
                "command": "\"/mnt/c/Users/me/AppData/Local/Programs/VibeBuddy/vibebuddy-hook.exe\" codex",
                "commandWindows": r#"& "C:\Users\me\AppData\Local\Programs\VibeBuddy\vibebuddy-hook.exe" codex"#,
                "timeout": 2,
            })
        );
        assert!(Call::of(&call.entry()) == Some(call));
    }

    #[test]
    fn only_a_drive_path_is_moved_under_mnt() {
        assert_eq!(wsl_path(r"D:\Tools\vibebuddy-hook.exe"), "/mnt/d/Tools/vibebuddy-hook.exe");
        assert_eq!(wsl_path(r"\\server\share\vibebuddy-hook.exe"), r"\\server\share\vibebuddy-hook.exe");
        assert_eq!(wsl_path("/home/me/vibebuddy-hook"), "/home/me/vibebuddy-hook");
    }

    #[test]
    fn codex_untrusted_means_none_of_our_entries_has_a_trusted_hash() {
        let wanted = wanted(BINARY, Agent::Codex);
        let source = "/home/me/.codex/hooks.json".to_owned();
        // Another tool's hook comes first in Stop, so ours is that event's second group.
        let theirs = json!({ "hooks": [{ "type": "command", "command": "other-tool" }] });
        let mut root = install(Map::new(), Agent::Codex, BINARY);
        root["hooks"]["Stop"].as_array_mut().unwrap().insert(0, theirs);
        let untrusted = |config: &str| none_trusted(&root, &wanted, std::slice::from_ref(&source), config);

        let others = "[features]\nhooks = true\n\n[hooks.state.\"omo@x:hooks/a.json:stop:0:0\"]\ntrusted_hash = \"sha256:aa\"\n";
        assert!(untrusted(""), "no config state at all");
        assert!(untrusted(others), "only another tool's hooks are trusted");
        assert!(untrusted("[hooks.state.\"/home/me/.codex/hooks.json:stop:0:0\"]\ntrusted_hash = \"sha256:bb\"\n"), "that is the other tool's group");
        assert!(!untrusted(&format!("{others}\n[hooks.state.\"/home/me/.codex/hooks.json:stop:1:0\"]\ntrusted_hash = \"sha256:cc\"\n")));
        assert!(!untrusted("[hooks.state.'/home/me/.codex/hooks.json:user_prompt_submit:0:0']\ntrusted_hash = 'sha256:dd'\n"));
        assert!(!untrusted("bypass_hook_trust = true\n"), "Codex doesn't check trust at all");
        assert!(untrusted("[profiles.x]\nbypass_hook_trust = true\n"), "only the top-level setting counts");
        assert!(!none_trusted(&Map::new(), &wanted, &[source], ""), "nothing of ours to trust");
    }

    #[test]
    fn a_windows_hook_key_is_read_with_its_escapes() {
        let (_, trusted) = trusted_hook_keys("[hooks.state.\"C:\\\\Users\\\\me\\\\.codex\\\\hooks.json:session_end:0:0\"]\ntrusted_hash = \"x\"\n");
        assert!(trusted.contains(r"C:\Users\me\.codex\hooks.json:session_end:0:0"), "{trusted:?}");
        assert_eq!(snake_case("PermissionRequest"), "permission_request");
    }

    #[test]
    fn uninstalling_drops_what_it_empties() {
        assert_eq!(
            uninstall(install(Map::new(), Agent::Codex, BINARY)),
            Map::new()
        );
    }

    #[test]
    fn unrecognised_shapes_are_left_alone() {
        let root =
            object(json!({ "hooks": { "Stop": "not a list", "Start": [{ "matcher": "x" }] } }));
        assert_eq!(uninstall(root.clone()), root);
    }

    #[test]
    fn the_diff_names_each_event_as_the_mac_does() {
        let before = object(json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "other-tool" }] }] } }));
        let after = install(before.clone(), Agent::Codex, BINARY);
        let lines = describe_change(&before, &after);
        assert_eq!(lines.len(), Agent::Codex.events().len());
        assert!(lines.contains(&format!("+ Stop: {}", codex_shown())));
        assert_eq!(describe_change(&after, &uninstall(after.clone())).first().map(|line| &line[..2]), Some("- "));
        assert!(describe_change(&after, &after).is_empty());
    }

    #[test]
    fn an_owned_file_shows_whole() {
        assert_eq!(describe_owned_file(None, Some("a\n\nb\n")), ["+ a", "+ b"]);
        assert_eq!(describe_owned_file(Some("a\n"), None), ["- a"]);
        assert!(describe_owned_file(Some("a"), Some("a")).is_empty());
    }

    #[test]
    fn an_owned_file_is_installed_with_a_windows_path() {
        let home = std::env::temp_dir().join(format!("vibebuddy-hook-owned-{}", std::process::id()));
        let binary = r"C:\Users\me\AppData\Local\Programs\VibeBuddy\vibebuddy-hook.exe";
        for agent in [cli_agents::Agent::Copilot, cli_agents::Agent::OpenCode, cli_agents::Agent::Pi] {
            let target = Target::Owned(agent);
            assert!(!is_installed(target, &home, binary));
            apply(&plan(target, &home, binary, true).unwrap(), &home).unwrap();
            assert!(is_installed(target, &home, binary), "{}", target.argument());
            assert!(!is_installed(target, &home, r"C:\elsewhere\vibebuddy-hook.exe"));
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn installed_means_every_event_runs_this_binary() {
        let home = std::env::temp_dir().join(format!("vibebuddy-hook-status-{}", std::process::id()));
        let target = Target::Merged(Agent::Claude);
        let path = target.path(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(!is_installed(target, &home, BINARY));
        let change = plan(target, &home, BINARY, true).unwrap();
        apply(&change, &home).unwrap();
        assert!(is_installed(target, &home, BINARY));
        assert!(!is_installed(target, &home, "/elsewhere/vibebuddy-hook"));
        assert!(plan(target, &home, BINARY, true).unwrap().lines.is_empty());
        let _ = std::fs::remove_dir_all(home);
    }
}
