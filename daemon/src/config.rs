//! daemon 持有的配置：App 通过接口读写，不直接碰文件；`cargo run` 单独跑时
//! 也有配置可读。环境变量仍是开发时的覆盖手段，优先级高于文件。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// 最近一次写进设备的播报音色 id；None 表示还没写过，设备用内置音色。
    pub voice: Option<String>,
    /// 链路异常时 App 是否弹 macOS 通知。
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

/// 配置文件位置：环境变量 `BEACON_CONFIG_FILE` 优先，否则放在 Application
/// Support 里 `stats.json` 旁边。没有 `HOME` 时不落盘，配置只在内存里。
pub fn config_file() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("BEACON_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/AgentBeacon/config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_broken_file_yields_defaults() {
        let dir = std::env::temp_dir().join(format!("beacon-config-{}", std::process::id()));
        let path = dir.join("config.json");
        assert_eq!(Config::load(&path), Config::default());
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::write(&path, "not json").expect("写坏文件");
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
        config.save(&path).expect("保存配置");
        assert_eq!(Config::load(&path), config);
        let _ = std::fs::remove_dir_all(dir);
    }
}
