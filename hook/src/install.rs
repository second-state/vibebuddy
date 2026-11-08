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

/// One agent VibeBuddy connects to: entries merged into a config the agent shares with the user, or a whole file
/// of VibeBuddy's own. In the Mac app's order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Merged(Agent),
    Owned(cli_agents::Agent),
    Kimi,
}

impl Target {
    const ALL: [Target; 6] = [
        Target::Merged(Agent::Codex),
        Target::Merged(Agent::Claude),
        Target::Owned(cli_agents::Agent::OpenCode),
        Target::Owned(cli_agents::Agent::Copilot),
        Target::Owned(cli_agents::Agent::Pi),
        Target::Owned(cli_agents::Agent::Zcode),
    ];

    fn from_argument(argument: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|target| target.argument() == argument)
    }

    fn argument(self) -> &'static str {
        match self {
            Target::Merged(agent) => agent.argument(),
            Target::Owned(agent) => agent.argument(),
            Target::Kimi => "kimi",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Target::Merged(agent) => agent.display_name(),
            Target::Owned(agent) => agent.display_name(),
            Target::Kimi => "Kimi Code CLI",
        }
    }

    /// Exists once the agent has run on this machine.
    fn dir(self, home: &Path) -> PathBuf {
        match self {
            Target::Merged(agent) => home.join(agent.dir()),
            Target::Owned(agent) => agent.dir(home),
            Target::Kimi => std::env::var_os("KIMI_CODE_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join(".kimi-code")),
        }
    }

    fn path(self, home: &Path) -> PathBuf {
        match self {
            Target::Merged(agent) => home.join(agent.dir()).join(agent.config_file()),
            Target::Owned(agent) => agent.file(home),
            Target::Kimi => self.dir(home).join("config.toml"),
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
    Zcode(crate::zcode::Plan),
    Kimi { before: Option<String>, after: String },
    /// None deletes the file.
    Owned(Option<String>),
}

fn plan(target: Target, home: &Path, binary: &str, installing: bool) -> Result<Change, String> {
    let path = target.path(home);
    let (after, lines) = match target {
        Target::Merged(agent) => {
            let before = read(&path)?;
            let after = if installing {
                install(before.clone(), agent, binary)
            } else {
                uninstall(before.clone())
            };
            let lines = describe_change(&before, &after);
            (Contents::Merged(after), lines)
        }
        Target::Owned(cli_agents::Agent::Zcode) => {
            let plan = crate::zcode::plan(home, binary, installing)?;
            let lines = plan.lines.clone();
            (Contents::Zcode(plan), lines)
        }
        Target::Owned(agent) => {
            let before = std::fs::read_to_string(&path).ok();
            let after = installing.then(|| agent.file_contents(binary));
            let lines = describe_owned_file(before.as_deref(), after.as_deref());
            (Contents::Owned(after), lines)
        }
        Target::Kimi => {
            let before = read_optional(&path)?;
            let after = kimi_config(before.as_deref().unwrap_or(""), binary, installing)?;
            // Kimi's config may contain API keys: never expose the rest of the file in the preview.
            let lines = if before.as_deref().unwrap_or("") == after {
                Vec::new()
            } else {
                let sign = if installing { '+' } else { '-' };
                KIMI_EVENTS
                    .iter()
                    .map(|event| format!("{sign} {event}: VibeBuddy hook"))
                    .collect()
            };
            (Contents::Kimi { before, after }, lines)
        }
    };
    Ok(Change {
        target,
        path,
        after,
        lines,
    })
}

fn apply(change: &Change, home: &Path) -> Result<(), String> {
    apply_with(change, home, || {})
}

/// `apply`, with `before_commit` run once a Kimi config is staged and checked, just before the final check. The
/// tests use it to change the file at that moment; nothing else calls it.
fn apply_with(
    change: &Change,
    home: &Path,
    before_commit: impl FnOnce(),
) -> Result<(), String> {
    let path = &change.path;
    let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
    match &change.after {
        Contents::Merged(root) => write(path, root),
        Contents::Zcode(plan) => plan.apply(),
        Contents::Kimi { before, after } => write_kimi(path, before.as_deref(), after, home, before_commit),
        Contents::Owned(Some(text)) => {
            std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(fail)?;
            std::fs::write(path, text).map_err(fail)
        }
        Contents::Owned(None) => std::fs::remove_file(path).map_err(fail),
    }
}

/// Kimi's config as it is now; `None` when there is no file yet.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Puts a planned Kimi config in place. The file must still hold what the plan read: it is checked before the new
/// text is staged and again just before the swap, so a change made in between is refused, not overwritten. The
/// backup holds exactly the text that was checked. A config that didn't exist is linked into place, which fails
/// rather than replacing a file that appeared meanwhile. A few microseconds remain between the last check and the
/// swap: POSIX offers no compare-and-swap for files.
fn write_kimi(
    path: &Path,
    before: Option<&str>,
    after: &str,
    home: &Path,
    before_commit: impl FnOnce(),
) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
    let changed = || format!("{} changed while planning; try again", path.display());
    if read_optional(path)?.as_deref() != before {
        return Err(changed());
    }
    std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(fail)?;
    // The config may hold API keys: the new file and the backup keep the old file's permissions.
    let permissions = match before {
        Some(_) => Some(std::fs::metadata(path).map_err(fail)?.permissions()),
        None => None,
    };
    let process = std::process::id();
    let staged = path.with_extension(format!("toml.vibebuddy-{process}.tmp"));
    let backup_staged = path.with_extension(format!("toml.bak.vibebuddy-{process}.tmp"));
    let result: Result<(), String> = (|| {
        create_private(&staged, after, permissions.clone()).map_err(fail)?;
        before_commit();
        if read_optional(path)?.as_deref() != before {
            return Err(changed());
        }
        match before {
            Some(text) => {
                create_private(&backup_staged, text, permissions).map_err(fail)?;
                std::fs::rename(&backup_staged, path.with_extension("toml.bak")).map_err(fail)?;
                std::fs::rename(&staged, path).map_err(fail)
            }
            None => std::fs::hard_link(&staged, path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    changed()
                } else {
                    fail(error)
                }
            }),
        }
    })();
    // Whatever was staged and not moved into place is removed; after a successful swap these are already gone.
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_file(&backup_staged);
    result
}

/// A new file holding `text`, created private so nothing else can read it while it is written, then given
/// `permissions` when it is to replace a file that had some. Never overwrites: `create_new`.
fn create_private(
    path: &Path,
    text: &str,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.sync_all()
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
                    .is_some_and(|groups| {
                        groups
                            .iter()
                            .any(|group| commands(group).any(|command| command == wanted))
                    })
            })
        }
        Target::Owned(cli_agents::Agent::Zcode) => crate::zcode::plan(home, binary, true).is_ok_and(|plan| plan.lines.is_empty()),
        Target::Owned(_) => std::fs::read_to_string(&path).is_ok_and(|text| text.contains(binary)),
        Target::Kimi => std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| {
                kimi_config(&text, binary, true)
                    .ok()
                    .map(|after| after == text)
            })
            .unwrap_or(false),
    }
}

fn describe_change(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let events = |root: &Map<String, Value>| -> std::collections::BTreeMap<String, Vec<String>> {
        root.get("hooks")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(event, groups)| {
                let commands = groups
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(commands)
                    .map(str::to_owned)
                    .collect();
                (event.clone(), commands)
            })
            .collect()
    };
    let (earlier, later) = (events(before), events(after));
    let names: std::collections::BTreeSet<&String> = earlier.keys().chain(later.keys()).collect();
    let mut lines = Vec::new();
    for event in names {
        let (old, new) = (
            earlier.get(event).cloned().unwrap_or_default(),
            later.get(event).cloned().unwrap_or_default(),
        );
        lines.extend(
            new.iter()
                .filter(|command| !old.contains(command))
                .map(|command| format!("+ {event}: {command}")),
        );
        lines.extend(
            old.iter()
                .filter(|command| !new.contains(command))
                .map(|command| format!("- {event}: {command}")),
        );
    }
    lines
}

fn describe_owned_file(before: Option<&str>, after: Option<&str>) -> Vec<String> {
    if before == after {
        return Vec::new();
    }
    let lines = |text: Option<&str>, sign: char| -> Vec<String> {
        text.unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| format!("{sign} {line}"))
            .collect()
    };
    [lines(before, '-'), lines(after, '+')].concat()
}

const KIMI_EVENTS: [&str; 7] = [
    "UserPromptSubmit",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "Interrupt",
    "SessionEnd",
];

fn kimi_config(text: &str, binary: &str, installing: bool) -> Result<String, String> {
    use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| format!("invalid Kimi TOML: {error}"))?;
    let quoted = format!("'{}' kimi", binary.replace('\'', "'\\''"));
    if document.get("hooks").is_none() {
        if !installing {
            return Ok(text.to_owned());
        }
        document["hooks"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let hooks = document["hooks"]
        .as_array_of_tables_mut()
        .ok_or("Kimi hooks must be an array of tables")?;
    let ours = |table: &Table| {
        table
            .get("command")
            .and_then(Item::as_str)
            .and_then(|command| command.strip_prefix('\'')?.strip_suffix("' kimi"))
            .map(|path| path.replace("'\\''", "'"))
            .is_some_and(|path| {
                Path::new(&path).is_absolute()
                    && Path::new(&path)
                        .file_name()
                        .is_some_and(|name| name == "vibebuddy-hook")
            })
    };
    let complete = KIMI_EVENTS.iter().all(|event| {
        hooks.iter().any(|table| {
            table.get("event").and_then(Item::as_str) == Some(*event)
                && table.get("command").and_then(Item::as_str) == Some(quoted.as_str())
        })
    });
    if installing && complete {
        return Ok(text.to_owned());
    }
    if !installing && !hooks.iter().any(ours) {
        return Ok(text.to_owned());
    }
    hooks.retain(|table| !ours(table));
    if installing {
        for event in KIMI_EVENTS {
            let mut table = Table::new();
            table["event"] = value(event);
            table["command"] = value(quoted.as_str());
            table["timeout"] = value(2);
            hooks.push(table);
        }
    }
    Ok(document.to_string())
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| "HOME is not set".to_owned())
}

/// The path written into configs: this binary, wherever it was installed.
fn binary() -> Result<String, String> {
    let binary =
        std::env::current_exe().map_err(|error| format!("cannot find this binary: {error}"))?;
    binary
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "this binary's path is not valid UTF-8".to_owned())
}

/// `vibebuddy-hook install|uninstall [agent]`: every agent that has run here, or just the one named.
pub fn run(installing: bool, only: Option<&str>) -> Result<(), String> {
    let (home, binary) = (home()?, binary()?);
    let targets: Vec<Target> = match only {
        Some(argument) => vec![
            Target::from_argument(argument).ok_or_else(|| format!("unknown agent {argument}"))?,
        ],
        None => Target::ALL
            .into_iter()
            .filter(|target| target.dir(&home).is_dir())
            .collect(),
    };
    if targets.is_empty() {
        println!(
            "None of Claude Code, Codex, OpenCode, GitHub Copilot CLI, Pi or ZCode has run here yet. Run one once, then run this again."
        );
    }
    for target in targets {
        if !target.dir(&home).is_dir() {
            return Err(format!(
                "{} wasn't found on this computer",
                target.display_name()
            ));
        }
        let change = plan(target, &home, &binary, installing)?;
        if change.lines.is_empty() {
            println!(
                "{}: nothing to change in {}",
                target.display_name(),
                change.path.display()
            );
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
            (Contents::Zcode(_), _) => println!("{name}: updated {} and VibeBuddy local plugin", change.path.display()),
            (Contents::Owned(Some(_)), _) => println!("{name}: wrote {}", change.path.display()),
            _ => println!("{name}: removed {}", change.path.display()),
        }
        if installing && change.target == Target::Merged(Agent::Codex) {
            println!(
                "  Codex runs changed hooks only after you trust them: open /hooks in Codex and trust the VibeBuddy entries."
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
    let target =
        Target::from_argument(argument).ok_or_else(|| format!("unknown agent {argument}"))?;
    let change = plan(target, &home()?, &binary()?, installing)?;
    serde_json::to_string(&json!({ "config": change.path, "lines": change.lines }))
        .map_err(|error| error.to_string())
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

    #[test]
    fn kimi_config_preserves_other_hooks_and_is_idempotent() {
        let original = "# user config\nmodel = \"example\"\n\n[[hooks]]\nevent = \"Stop\"\ncommand = \"other\"\n";
        let installed = kimi_config(original, "/tmp/a'b/vibebuddy-hook", true).unwrap();
        assert!(installed.contains("# user config"));
        assert!(installed.contains("command = \"other\""));
        assert_eq!(
            kimi_config(&installed, "/tmp/a'b/vibebuddy-hook", true).unwrap(),
            installed
        );
        let removed = kimi_config(&installed, "/tmp/a'b/vibebuddy-hook", false).unwrap();
        assert!(removed.contains("command = \"other\""));
        assert!(!removed.contains("vibebuddy-hook"));
        assert!(kimi_config("[[hooks", "/tmp/vibebuddy-hook", true).is_err());
        let repaired = kimi_config(&installed, "/new/vibebuddy-hook", true).unwrap();
        assert!(!repaired.contains("a'b"));
        assert_eq!(
            repaired.matches("/new/vibebuddy-hook").count(),
            KIMI_EVENTS.len()
        );
    }

    #[test]
    #[cfg(unix)]
    fn kimi_apply_refuses_changed_config_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "vibebuddy-kimi-install-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let before = "model = \"example\"\n".to_owned();
        std::fs::write(&path, &before).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let change = Change {
            target: Target::Kimi,
            path: path.clone(),
            after: Contents::Kimi {
                before: Some(before.clone()),
                after: kimi_config(&before, BINARY, true).unwrap(),
            },
            lines: vec!["change".into()],
        };
        std::fs::write(&path, "model = \"changed\"\n").unwrap();
        assert!(apply(&change, &root).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "model = \"changed\"\n"
        );
        std::fs::write(&path, &before).unwrap();
        apply(&change, &root).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_to_string(root.join("config.toml.bak")).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn kimi_command_quotes_shell_metacharacters() {
        let binary = "/tmp/a ' $HOME; space/vibebuddy-hook";
        let config = kimi_config("", binary, true).unwrap();
        let document = config.parse::<toml_edit::DocumentMut>().unwrap();
        let command = document["hooks"]
            .as_array_of_tables()
            .unwrap()
            .get(0)
            .unwrap()["command"]
            .as_str()
            .unwrap();
        let script = format!("set -- {command}; printf '%s\\n%s' \"$1\" \"$2\"");
        let output = std::process::Command::new("sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{binary}\nkimi")
        );
    }

    #[test]
    #[cfg(unix)]
    fn kimi_missing_config_is_created_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "vibebuddy-kimi-missing-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let change = Change {
            target: Target::Kimi,
            path: path.clone(),
            after: Contents::Kimi {
                before: None,
                after: kimi_config("", BINARY, true).unwrap(),
            },
            lines: vec!["change".into()],
        };
        apply(&change, &root).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!root.join("config.toml.bak").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A scratch directory of this test run, empty.
    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vibebuddy-kimi-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn kimi_change(path: &Path, before: Option<&str>) -> Change {
        Change {
            target: Target::Kimi,
            path: path.to_path_buf(),
            after: Contents::Kimi {
                before: before.map(str::to_owned),
                after: kimi_config(before.unwrap_or(""), BINARY, true).unwrap(),
            },
            lines: vec!["change".into()],
        }
    }

    /// Names of the files in `root` other than `config.toml`: staged leftovers and backups.
    fn other_files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "config.toml")
            .collect();
        names.sort();
        names
    }

    /// Every top-level setting except `hooks`, as text: what installing and removing must leave alone.
    fn settings_except_hooks(text: &str) -> Vec<(String, String)> {
        let document = text.parse::<toml_edit::DocumentMut>().unwrap();
        document
            .iter()
            .filter(|(key, _)| *key != "hooks")
            .map(|(key, item)| (key.to_owned(), item.to_string()))
            .collect()
    }

    #[test]
    fn kimi_install_leaves_every_other_setting_as_it_was() {
        let original = concat!(
            "# Kimi settings, kept as written.\n",
            "default_model = \"smoke\"   # the model to use\n",
            "auto_session_title = false\n",
            "\n",
            "[providers.smoke]\n",
            "type = \"anthropic\"\n",
            "api_key = \"local-test-only\"   # never shown in the preview\n",
            "base_url = \"http://127.0.0.1:9\"\n",
            "\n",
            "[models.smoke]\n",
            "provider = \"smoke\"\n",
            "max_context_size = 100000\n",
            "\n",
            "[[hooks]]\n",
            "event = \"Stop\"\n",
            "command = \"/usr/bin/other-tool notify\"\n",
            "timeout = 5\n",
        );
        let installed = kimi_config(original, BINARY, true).unwrap();
        assert_eq!(
            settings_except_hooks(&installed),
            settings_except_hooks(original)
        );
        assert!(installed.contains("# never shown in the preview"));
        assert!(installed.contains("command = \"/usr/bin/other-tool notify\""));
        let hooks = installed.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            hooks["hooks"].as_array_of_tables().unwrap().len(),
            KIMI_EVENTS.len() + 1
        );
        let removed = kimi_config(&installed, BINARY, false).unwrap();
        assert_eq!(settings_except_hooks(&removed), settings_except_hooks(original));
        assert!(removed.contains("command = \"/usr/bin/other-tool notify\""));
        assert!(!removed.contains("vibebuddy-hook"));
    }

    #[test]
    fn kimi_apply_refuses_a_change_made_after_staging() {
        let root = scratch("staged-change");
        let path = root.join("config.toml");
        let before = "model = \"example\"\n";
        std::fs::write(&path, before).unwrap();
        let change = kimi_change(&path, Some(before));
        let result = apply_with(&change, &root, || {
            std::fs::write(&path, "model = \"changed by Kimi\"\n").unwrap();
        });
        assert!(result.is_err(), "the write went ahead over a concurrent change");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "model = \"changed by Kimi\"\n"
        );
        assert!(other_files(&root).is_empty(), "left behind: {:?}", other_files(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kimi_apply_does_not_replace_a_config_created_after_staging() {
        let root = scratch("created-change");
        let path = root.join("config.toml");
        let change = kimi_change(&path, None);
        let result = apply_with(&change, &root, || {
            std::fs::write(&path, "model = \"created by Kimi\"\n").unwrap();
        });
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "model = \"created by Kimi\"\n"
        );
        assert!(other_files(&root).is_empty(), "left behind: {:?}", other_files(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn kimi_backup_is_private_and_holds_the_checked_text() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("backup");
        let path = root.join("config.toml");
        let before = "api_key = \"local-test-only\"\n";
        std::fs::write(&path, before).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        apply(&kimi_change(&path, Some(before)), &root).unwrap();
        let backup = root.join("config.toml.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), before);
        assert_eq!(std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(other_files(&root) == ["config.toml.bak"], "{:?}", other_files(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

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
        let before =
            object(json!({ "hooks": { "Stop": [{ "hooks": [{ "command": "other-tool" }] }] } }));
        let after = install(before.clone(), Agent::Codex, BINARY);
        let lines = describe_change(&before, &after);
        assert_eq!(lines.len(), Agent::Codex.events().len());
        assert!(lines.contains(&format!("+ Stop: \"{BINARY}\" codex")));
        assert_eq!(
            describe_change(&after, &uninstall(after.clone()))
                .first()
                .map(|line| &line[..2]),
            Some("- ")
        );
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
        let home =
            std::env::temp_dir().join(format!("vibebuddy-hook-status-{}", std::process::id()));
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
