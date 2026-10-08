//! `vibebuddy-hook install` / `uninstall`: where there is no app (Linux), the hook registers itself in the
//! user-level config of Claude Code and Codex, and writes or deletes the file it owns for OpenCode and Copilot
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
    const ALL: [Agent; 2] = [Agent::Claude, Agent::Codex];

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

pub fn run(installing: bool) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let binary =
        std::env::current_exe().map_err(|error| format!("cannot find this binary: {error}"))?;
    let binary = binary
        .to_str()
        .ok_or("this binary's path is not valid UTF-8")?;
    let mut found = false;
    for agent in Agent::ALL {
        let dir = home.join(agent.dir());
        if !dir.is_dir() {
            continue;
        }
        found = true;
        let path = dir.join(agent.config_file());
        let before = read(&path)?;
        let after = if installing {
            install(before.clone(), agent, binary)
        } else {
            uninstall(before.clone())
        };
        if after == before {
            println!(
                "{}: nothing to change in {}",
                agent.display_name(),
                path.display()
            );
            continue;
        }
        write(&path, &after)?;
        println!(
            "{}: updated {} (the old version is in {}.bak)",
            agent.display_name(),
            path.display(),
            agent.config_file()
        );
        if installing && agent == Agent::Codex {
            println!(
                "  Codex runs changed hooks only after you trust them: open /hooks in Codex and trust the Vibe Buddy entries."
            );
        }
    }
    for agent in cli_agents::Agent::ALL {
        if !agent.dir(&home).is_dir() {
            continue;
        }
        found = true;
        let path = agent.file(&home);
        let wanted = installing.then(|| agent.file_contents(binary));
        let current = std::fs::read_to_string(&path).ok();
        if current == wanted {
            println!("{}: nothing to change in {}", agent.display_name(), path.display());
            continue;
        }
        let fail = |error: std::io::Error| format!("cannot write {}: {error}", path.display());
        match wanted {
            Some(text) => {
                std::fs::create_dir_all(path.parent().unwrap_or(&home)).map_err(fail)?;
                std::fs::write(&path, text).map_err(fail)?;
                println!("{}: wrote {}", agent.display_name(), path.display());
            }
            None => {
                std::fs::remove_file(&path).map_err(fail)?;
                println!("{}: removed {}", agent.display_name(), path.display());
            }
        }
    }
    if !found {
        println!(
            "None of Claude Code, Codex, OpenCode or GitHub Copilot CLI has run here yet. Run one once, then run this again."
        );
    }
    Ok(())
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
}
