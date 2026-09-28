//! Config owned by the daemon: the app reads and writes it through the API, never the file; `cargo run`
//! on its own still has config to read. Environment variables remain the dev override and beat the file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Id of the announcement voice last written to the device; None means never written, so the built-in voice.
    pub voice: Option<String>,
    /// Whether the app shows a macOS notification when the link has trouble.
    pub notify_link: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            voice: None,
            notify_link: true,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }
}

/// Config file location: `VIBEBUDDY_CONFIG_FILE` wins, otherwise next to `stats.json` in Application
/// Support. Without `HOME` nothing is written and config lives only in memory.
pub fn config_file() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VIBEBUDDY_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/VibeBuddy/config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_broken_file_yields_defaults() {
        let dir = std::env::temp_dir().join(format!("beacon-config-{}", std::process::id()));
        let path = dir.join("config.json");
        assert_eq!(Config::load(&path), Config::default());
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(&path, "not json").expect("write broken file");
        assert_eq!(Config::load(&path), Config::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_config_round_trips() {
        let dir = std::env::temp_dir().join(format!("beacon-config-rt-{}", std::process::id()));
        let path = dir.join("nested").join("config.json");
        let config = Config {
            voice: Some("wanwanxiaohe".to_owned()),
            notify_link: false,
        };
        config.save(&path).expect("save config");
        assert_eq!(Config::load(&path), config);
        let _ = std::fs::remove_dir_all(dir);
    }
}
