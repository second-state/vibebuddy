//! VibeBuddy's local ZCode plugin and its isolated user-config registration.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
const ID: &str = "vibebuddy@inline";

pub fn manifest(binary: &str) -> Value {
    let mut hooks = serde_json::Map::new();
    for event in [
        "UserPromptSubmit",
        "PostToolUse",
        "PermissionRequest",
        "PostToolUseFailure",
        "Stop",
    ] {
        hooks.insert(event.into(), json!([{"hooks": [{"type": "process", "command": binary, "args": ["zcode"], "timeoutMs": 2000}]}]));
    }
    json!({"name": "vibebuddy", "version": "1.0.0", "description": "VibeBuddy Agent status", "hooks": hooks})
}
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}
pub struct Plan {
    config: PathBuf,
    plugin: PathBuf,
    before: Option<String>,
    plugin_before: Option<String>,
    after: String,
    plugin_after: Option<String>,
    pub lines: Vec<String>,
}
pub fn plan(home: &Path, binary: &str, installing: bool) -> Result<Plan, String> {
    let base = home.join(".zcode/cli");
    let config = base.join("config.json");
    let root = base.join("vibebuddy-plugin");
    let plugin = root.join(".zcode-plugin/plugin.json");
    let before = read(&config)?;
    let plugin_before = read(&plugin)?;
    let mut value: Value = serde_json::from_str(before.as_deref().unwrap_or("{}"))
        .map_err(|_| "invalid ZCode config.json".to_owned())?;
    let object = value
        .as_object_mut()
        .ok_or("ZCode config must be an object")?;
    if !installing
        && plugin_before.is_none()
        && !object.get("plugins").is_some_and(|p| {
            p.get("dirs")
                .and_then(Value::as_array)
                .is_some_and(|dirs| dirs.contains(&Value::from(root.to_string_lossy().as_ref())))
                || p.get("enabledPlugins").and_then(|v| v.get(ID)).is_some()
        })
    {
        return Ok(Plan {
            config,
            plugin,
            before: before.clone(),
            plugin_before,
            after: before.unwrap_or_default(),
            plugin_after: None,
            lines: vec![],
        });
    }
    // ZCode forces the runner on for plugin hooks. Reject dormant user events whose
    // enabled flag was omitted, since enabling the runner could activate those too.
    if installing
        && object.get("hooks").is_some_and(|h| {
            h.get("enabled").is_none()
                && h.get("events")
                    .and_then(Value::as_object)
                    .is_some_and(|events| !events.is_empty())
        })
    {
        return Err("ZCode has hook events without an explicit enabled setting; set hooks.enabled in ZCode before connecting".into());
    }
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("ZCode plugins must be an object")?;
    if installing && plugins.get("enabled") == Some(&Value::Bool(false)) {
        return Err("ZCode plugins are disabled; enable plugins in ZCode before connecting".into());
    }
    let dirs = plugins
        .entry("dirs")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("ZCode plugins.dirs must be an array")?;
    if dirs.iter().any(|v| !v.is_string()) {
        return Err("ZCode plugins.dirs must contain paths".into());
    }
    let root_text = root.to_str().ok_or("ZCode plugin path is not UTF-8")?;
    dirs.retain(|v| v.as_str() != Some(root_text));
    if installing {
        dirs.push(Value::from(root_text));
    }
    let enabled = plugins
        .entry("enabledPlugins")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("ZCode enabledPlugins must be an object")?;
    if installing {
        enabled.insert(ID.into(), Value::Bool(true));
    } else {
        enabled.remove(ID);
    }
    if let Some(text) = &plugin_before {
        let old: Value = serde_json::from_str(text)
            .map_err(|_| "VibeBuddy's ZCode plugin manifest is invalid")?;
        if old.get("name").and_then(Value::as_str) != Some("vibebuddy") {
            return Err("ZCode plugin path belongs to another plugin".into());
        }
    }
    let plugin_after =
        installing.then(|| serde_json::to_string_pretty(&manifest(binary)).unwrap() + "\n");
    let config_changed = before
        .as_ref()
        .map(|s| serde_json::from_str::<Value>(s).ok())
        != Some(Some(value.clone()));
    let changed =
        (installing || before.is_some()) && config_changed || plugin_after != plugin_before;
    let after = if !config_changed {
        before.clone().unwrap_or_default()
    } else {
        serde_json::to_string_pretty(&value).unwrap() + "\n"
    };
    Ok(Plan {
        config,
        plugin,
        before,
        plugin_before,
        after,
        plugin_after,
        lines: if changed {
            vec![format!(
                "{} VibeBuddy local ZCode plugin and registration",
                if installing { "+" } else { "-" }
            )]
        } else {
            vec![]
        },
    })
}
impl Plan {
    pub fn apply(&self) -> Result<(), String> {
        if self.lines.is_empty() {
            return Ok(());
        }
        if read(&self.config)? != self.before || read(&self.plugin)? != self.plugin_before {
            return Err("ZCode configuration changed while planning; try again".into());
        }
        if let Some(text) = &self.plugin_after {
            replace(&self.plugin, self.plugin_before.as_deref(), text, false)?;
        }
        if self.before.as_deref() != Some(self.after.as_str()) {
            replace(&self.config, self.before.as_deref(), &self.after, true)?;
        }
        if self.plugin_after.is_none() && self.plugin_before.is_some() {
            if read(&self.plugin)? != self.plugin_before {
                return Err("ZCode plugin changed while planning; try again".into());
            }
            std::fs::remove_file(&self.plugin).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
fn replace(path: &Path, before: Option<&str>, after: &str, backup: bool) -> Result<(), String> {
    use std::io::Write;
    let result = || -> Result<(), String> {
        std::fs::create_dir_all(path.parent().ok_or("missing parent directory")?)
            .map_err(|e| e.to_string())?;
        let temp = path.with_extension(format!("vibebuddy-{}.tmp", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let staged = (|| -> Result<(), String> {
            let mut file = options.open(&temp).map_err(|e| e.to_string())?;
            file.write_all(after.as_bytes())
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            if read(path)?.as_deref() != before {
                return Err("ZCode config changed while planning; try again".into());
            }
            if let Some(text) = before {
                if backup {
                    replace(
                        &path.with_extension("json.bak"),
                        read(&path.with_extension("json.bak"))?.as_deref(),
                        text,
                        false,
                    )?;
                }
                if path.extension().is_none_or(|ext| ext != "bak") {
                    std::fs::set_permissions(
                        &temp,
                        std::fs::metadata(path)
                            .map_err(|e| e.to_string())?
                            .permissions(),
                    )
                    .map_err(|e| e.to_string())?;
                }
                if read(path)?.as_deref() != before {
                    return Err("ZCode config changed while planning; try again".into());
                }
                std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
            } else {
                std::fs::hard_link(&temp, path).map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(temp);
        staged
    };
    result()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_preserves_other_hooks_and_rejects_stale_plans() {
        let home = std::env::temp_dir().join(format!("vibebuddy-zcode-{}", std::process::id()));
        let base = home.join(".zcode/cli");
        std::fs::create_dir_all(&base).unwrap();
        let config = base.join("config.json");
        let original = json!({"hooks":{"enabled":false,"events":{"Stop":[]}}, "plugins":{"dirs":["/other"],"enabledPlugins":{"other@inline":false}},"secret":"PRIVATE"});
        std::fs::write(&config, original.to_string()).unwrap();
        let planned = plan(&home, "/with spaces/vibebuddy-hook", true).unwrap();
        assert!(!planned.lines.join("").contains("PRIVATE"));
        planned.apply().unwrap();
        let installed: Value = serde_json::from_str(&read(&config).unwrap().unwrap()).unwrap();
        assert_eq!(installed["hooks"], original["hooks"]);
        assert_eq!(installed["secret"], original["secret"]);
        assert_eq!(
            installed["plugins"]["enabledPlugins"]["other@inline"],
            false
        );
        assert!(
            plan(&home, "/with spaces/vibebuddy-hook", true)
                .unwrap()
                .lines
                .is_empty()
        );
        let stale = plan(&home, "/new/vibebuddy-hook", true).unwrap();
        std::fs::write(&config, installed.to_string() + "\n").unwrap();
        assert!(stale.apply().is_err());
        plan(&home, "/new/vibebuddy-hook", false)
            .unwrap()
            .apply()
            .unwrap();
        let removed: Value = serde_json::from_str(&read(&config).unwrap().unwrap()).unwrap();
        assert_eq!(removed["plugins"], original["plugins"]);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn zcode_events_forward_only_identity_and_classification() {
        use crate::cli_agents::zcode_payload;
        for (event, expected) in [
            ("UserPromptSubmit", "working"),
            ("PermissionRequest", "needs_input"),
            ("PostToolUse", "working"),
            ("PostToolUseFailure", "working"),
            ("Stop", "done"),
        ] {
            let output = zcode_payload(&json!({"hookEventName":event,"sessionId":"s","turnId":"t","cwd":"/work","prompt":"SECRET","responseText":"All done.","toolInput":{"secret":"SECRET"},"transcriptPath":"SECRET"})).unwrap();
            assert_eq!(output["event"], expected);
            assert_eq!(output["turn_id"], "t");
            assert!(!serde_json::to_string(&output).unwrap().contains("SECRET"));
            assert!(output.keys().all(|k| {
                [
                    "agent",
                    "session_id",
                    "turn_id",
                    "cwd",
                    "event",
                    "response_kind",
                    "surface",
                    "host_bundle_id",
                    "host_ttys",
                    "host_focus_url",
                    "host_pids",
                    "tmux_socket",
                    "tmux_pane",
                ]
                .contains(&k.as_str())
            }));
        }
        assert!(zcode_payload(&json!({"hookEventName":"SessionStart","sessionId":"s"})).is_none());
        assert!(zcode_payload(&json!({"hookEventName":"Stop","sessionId":""})).is_none());
    }
    #[test]
    fn disabled_plugins_and_ambiguous_user_hooks_are_not_activated() {
        let home =
            std::env::temp_dir().join(format!("vibebuddy-zcode-disabled-{}", std::process::id()));
        let base = home.join(".zcode/cli");
        std::fs::create_dir_all(&base).unwrap();
        let config = base.join("config.json");
        assert!(plan(&home, "/hook", false).unwrap().lines.is_empty());
        assert!(!config.exists());
        for value in [
            json!({"plugins":{"enabled":false}}),
            json!({"hooks":{"events":{"Stop":[]}}}),
        ] {
            std::fs::write(&config, value.to_string()).unwrap();
            assert!(plan(&home, "/hook", true).is_err());
            assert_eq!(read(&config).unwrap().unwrap(), value.to_string());
        }
        std::fs::remove_file(&config).unwrap();
        plan(&home, "/hook", true).unwrap().apply().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
                0o600
            );
            plan(&home, "/new-hook", true).unwrap().apply().unwrap();
            // Force a config edit so a backup is written even when only the plugin changed.
            std::fs::write(&config, "{}").unwrap();
            plan(&home, "/hook", true).unwrap().apply().unwrap();
            assert_eq!(
                std::fs::metadata(config.with_extension("json.bak"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}
