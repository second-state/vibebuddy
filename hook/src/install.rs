//! `vibebuddy-hook install` / `uninstall`: where there is no app (Linux), the hook registers itself in the
//! user-level config of Claude Code and Codex, and writes or deletes the file it owns for OpenCode, Copilot and Pi
//! (`cli_agents`). Same rules as the app's `HookConfig.swift`: only Vibe Buddy's own
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

fn command(binary: &str, agent: Agent) -> String {
    format!("\"{binary}\" {}", agent.argument())
}

/// Adds our hook to every event that lacks it and drops our stale entries; everything else keeps its place.
fn install(root: Map<String, Value>, agent: Agent, binary: &str) -> Map<String, Value> {
    let wanted = command(binary, agent);
    let mut root = remove_where(root, |command| is_ours(command) && command != wanted);
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
            .any(|group| commands(group).any(|command| command == wanted))
        {
            groups
                .push(json!({ "hooks": [{ "type": "command", "command": wanted, "timeout": 2 }] }));
        }
    }
    root
}

fn uninstall(root: Map<String, Value>) -> Map<String, Value> {
    remove_where(root, is_ours)
}

/// Removes matching hook commands, then any group, event and `hooks` object left empty by that.
/// Shapes we don't recognise are someone else's and are left alone.
fn remove_where(
    mut root: Map<String, Value>,
    matches: impl Fn(&str) -> bool,
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
            entries.retain(|entry| {
                !entry
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(&matches)
            });
            !entries.is_empty()
        });
        !groups.is_empty()
    });
    if hooks.is_empty() {
        root.remove("hooks");
    }
    root
}

fn commands(group: &Value) -> impl Iterator<Item = &str> {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("command").and_then(Value::as_str))
}

/// One agent Vibe Buddy connects to: entries merged into a config the agent shares with the user, or a whole file
/// of Vibe Buddy's own. In the Mac app's order.
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
            let wanted = command(binary, agent);
            let Ok(root) = read(&path) else { return false };
            agent.events().iter().all(|event| {
                root.get("hooks")
                    .and_then(|hooks| hooks.get(*event))
                    .and_then(Value::as_array)
                    .is_some_and(|groups| groups.iter().any(|group| commands(group).any(|command| command == wanted)))
            })
        }
        Target::Owned(_) => std::fs::read_to_string(&path).is_ok_and(|text| text.contains(binary)),
    }
}

fn describe_change(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let events = |root: &Map<String, Value>| -> std::collections::BTreeMap<String, Vec<String>> {
        root.get("hooks")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(event, groups)| {
                let commands = groups.as_array().into_iter().flatten().flat_map(commands).map(str::to_owned).collect();
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
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| "HOME is not set".to_owned())
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
                "  Codex runs changed hooks only after you trust them: open /hooks in Codex and trust the Vibe Buddy entries."
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
            })
        })
        .collect();
    serde_json::to_string(&agents).map_err(|error| error.to_string())
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
            json!([{ "hooks": [{ "type": "command", "command": format!("\"{BINARY}\" codex"), "timeout": 2 }] }])
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
        let all: Vec<&str> = fresh["hooks"]
            .as_object()
            .expect("hooks")
            .values()
            .flat_map(|groups| groups.as_array().into_iter().flatten().flat_map(commands))
            .collect();
        assert!(
            all.iter()
                .all(|command| *command == format!("\"{BINARY}\" claude"))
        );
        assert_eq!(all.len(), Agent::Claude.events().len());
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
        assert!(lines.contains(&format!("+ Stop: \"{BINARY}\" codex")));
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
