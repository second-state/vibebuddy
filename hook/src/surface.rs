//! 判定 Agent 进程的运行处：跑在自己的桌面 App 里，跑在别的 App（终端、
//! 编辑器）里，还是没有宿主。
//!
//! 依据只有 `__CFBundleIdentifier`——LaunchServices 启动 App 时注入，沿进程
//! 链继承到 Hook。不看 `TERM_PROGRAM`：那要维护一张终端名到 bundle id 的
//! 映射表，用户换个没见过的终端就断。也不看 tty：Agent 执行工具命令用的是
//! 非交互子进程，即使宿主是终端也报 not a tty（2026-09-21 在 Ghostty 里实
//! 测 codex 确认）。

use serde_json::{Map, Value};

const BUNDLE_ID: &str = "__CFBundleIdentifier";

#[derive(Debug, PartialEq, Eq)]
pub enum Surface {
    /// 跑在 Agent 自己的桌面 App 里，K2 用该 Agent 的 deeplink。
    App,
    /// 跑在别的 App 里，这个 bundle id 就是 K2 的落点。未知的 App 一律按
    /// 宿主处理，所以没见过的终端也能跳对。
    Host(String),
    /// 没有宿主 App：SSH、守护进程、launchd 起的会话。K2 无处可去。
    Headless,
}

pub fn detect(own_bundle_id: &str) -> Surface {
    from_bundle_id(std::env::var(BUNDLE_ID).ok().as_deref(), own_bundle_id)
}

fn from_bundle_id(found: Option<&str>, own_bundle_id: &str) -> Surface {
    match found {
        Some(id) if id == own_bundle_id => Surface::App,
        Some(id) if !id.is_empty() => Surface::Host(id.to_owned()),
        _ => Surface::Headless,
    }
}

pub fn write_into(payload: &mut Map<String, Value>, surface: &Surface) {
    let name = match surface {
        Surface::App => "app",
        Surface::Host(bundle_id) => {
            payload.insert("host_bundle_id".to_owned(), Value::String(bundle_id.clone()));
            "host"
        }
        Surface::Headless => "headless",
    };
    payload.insert("surface".to_owned(), Value::String(name.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agents_own_app_is_not_a_host() {
        assert_eq!(from_bundle_id(Some("com.openai.codex"), "com.openai.codex"), Surface::App);
    }

    #[test]
    fn any_other_app_is_the_landing_spot() {
        assert_eq!(
            from_bundle_id(Some("com.mitchellh.ghostty"), "com.openai.codex"),
            Surface::Host("com.mitchellh.ghostty".to_owned())
        );
    }

    #[test]
    fn an_unknown_terminal_needs_no_code_change() {
        // 没见过的终端与见过的走同一条路：读到什么就跳到什么。
        assert_eq!(
            from_bundle_id(Some("net.example.SomeNewTerminal"), "com.openai.codex"),
            Surface::Host("net.example.SomeNewTerminal".to_owned())
        );
    }

    #[test]
    fn no_bundle_id_means_nowhere_to_go() {
        // SSH、launchd、守护进程起的 CLI 都落在这里。
        assert_eq!(from_bundle_id(None, "com.openai.codex"), Surface::Headless);
        assert_eq!(from_bundle_id(Some(""), "com.openai.codex"), Surface::Headless);
    }

    #[test]
    fn the_payload_carries_the_landing_spot_only_for_a_host() {
        let mut payload = Map::new();
        write_into(&mut payload, &Surface::Host("com.mitchellh.ghostty".to_owned()));
        assert_eq!(payload["surface"], "host");
        assert_eq!(payload["host_bundle_id"], "com.mitchellh.ghostty");

        let mut payload = Map::new();
        write_into(&mut payload, &Surface::App);
        assert_eq!(payload["surface"], "app");
        assert!(!payload.contains_key("host_bundle_id"));
    }
}
