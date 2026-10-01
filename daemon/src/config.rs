//! Config owned by the daemon: the app reads and writes it through the API, never the file; `cargo run`
//! on its own still has config to read. Environment variables remain the dev override and beat the file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Id of the announcement voice last written to the device; None means never written, so the built-in voice.
    pub voice: Option<String>,
    /// Whether the user is notified when the link has trouble: by the app on macOS, by the daemon elsewhere.
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

/// Config file location: `VIBEBUDDY_CONFIG_FILE` wins, otherwise `config.json` in [`config_dir`].
/// Without `HOME` nothing is written and config lives only in memory.
pub fn config_file() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VIBEBUDDY_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }
    Some(config_dir()?.join("config.json"))
}

/// Where the daemon keeps its config. On macOS it shares Application Support with the app; elsewhere it follows
/// the XDG base directories.
pub fn config_dir() -> Option<PathBuf> {
    app_dir("XDG_CONFIG_HOME", ".config")
}

/// Where the daemon keeps state such as today's stats; the same directory as [`config_dir`] on macOS.
pub fn state_dir() -> Option<PathBuf> {
    app_dir("XDG_STATE_HOME", ".local/state")
}

fn app_dir(xdg_variable: &str, xdg_default: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Application Support/VibeBuddy"));
    }
    xdg_dir(
        std::env::var_os(xdg_variable).map(PathBuf::from),
        home,
        xdg_default,
    )
}

/// The XDG spec says a relative value is invalid and must be ignored, so it falls back to the default under `HOME`.
fn xdg_dir(value: Option<PathBuf>, home: Option<PathBuf>, default: &str) -> Option<PathBuf> {
    value
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(default)))
        .map(|dir| dir.join("vibebuddy"))
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

    #[test]
    fn xdg_dirs_fall_back_to_home_unless_set_to_an_absolute_path() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            xdg_dir(None, home.clone(), ".config"),
            Some(PathBuf::from("/home/me/.config/vibebuddy"))
        );
        assert_eq!(
            xdg_dir(Some(PathBuf::from("/xdg/config")), home.clone(), ".config"),
            Some(PathBuf::from("/xdg/config/vibebuddy"))
        );
        assert_eq!(
            xdg_dir(Some(PathBuf::from("relative")), home, ".local/state"),
            Some(PathBuf::from("/home/me/.local/state/vibebuddy"))
        );
        assert_eq!(xdg_dir(None, None, ".config"), None);
    }
}
