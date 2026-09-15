//! 把聚合活动定位回 Mac 上的来源窗口。
//!
//! 所有参数都直接交给进程 API，不经过 shell。来源数据来自 Hook，仍按不可信
//! 输入处理：Codex thread id、Claude session id 与 GitHub repo 都先收窄字符集。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

use crate::activity::ActivitySource;

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const CODEX_BUNDLE_ID: &str = "com.openai.codex";
const CLAUDE_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// Claude App 记录每个 Code 会话的地方，一个账号一个子目录。
const DESKTOP_SESSIONS_DIR: &str = "Library/Application Support/Claude/claude-code-sessions";
/// 桌面会话 id 的前缀，deep link 只认这种形态。
const DESKTOP_ID_PREFIX: &str = "local_";

#[derive(Debug, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

/// 成功时返回实际打开的链接，方便日志说明 K2 到底跳去了哪里。
pub async fn open(source: ActivitySource) -> Result<String, String> {
    let desktop = match &source {
        ActivitySource::ClaudeCode { session_id, cwd } => desktop_sessions_dir()
            .and_then(|dir| desktop_session_id(&dir, session_id, cwd.as_deref())),
        _ => None,
    };
    let spec = command_for(&source, desktop.as_deref())?;
    let link = spec.args.last().cloned().unwrap_or_default();
    let output = tokio::time::timeout(
        OPEN_TIMEOUT,
        Command::new(spec.program).args(&spec.args).output(),
    )
    .await
    .map_err(|_| "打开来源超时".to_owned())?
    .map_err(|error| format!("无法启动打开命令：{error}"))?;
    if output.status.success() {
        return Ok(link);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("打开来源失败：{}", stderr.trim()))
}

fn command_for(source: &ActivitySource, desktop: Option<&str>) -> Result<CommandSpec, String> {
    match source {
        ActivitySource::Codex { thread_id } => {
            if !valid_identifier(thread_id) {
                return Err("Codex thread id 含有非法字符".to_owned());
            }
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec![
                    "-b".to_owned(),
                    CODEX_BUNDLE_ID.to_owned(),
                    format!("codex://threads/{thread_id}"),
                ],
            })
        }
        ActivitySource::ClaudeCode { session_id, .. } => {
            // 已经知道是哪个窗口时直接聚焦它。`claude://resume` 走的是另一条
            // 路：按 CLI session id 去磁盘上认领一份 transcript。同一个 id 在多
            // 个项目目录下各有一份时它只能挑一个，挑错了就打开一个内容陈旧的
            // 影子会话，而且每按一次都把整份 transcript 重新导入一遍。
            let link = match desktop {
                Some(desktop) => {
                    if !valid_desktop_id(desktop) {
                        return Err("桌面会话 id 格式无效".to_owned());
                    }
                    format!("claude://code/continue?session={desktop}")
                }
                // 终端里跑的 CLI 会话没有对应窗口，导入是唯一的打开方式。
                None => {
                    if !valid_identifier(session_id) {
                        return Err("Claude session id 含有非法字符".to_owned());
                    }
                    format!("claude://resume?session={session_id}")
                }
            };
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec!["-b".to_owned(), CLAUDE_BUNDLE_ID.to_owned(), link],
            })
        }
        ActivitySource::GitHubActions { repo, run_id } => {
            if !valid_repo(repo) {
                return Err("GitHub repo 格式无效".to_owned());
            }
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec![format!("https://github.com/{repo}/actions/runs/{run_id}")],
            })
        }
    }
}

/// Claude App 为每个 Code 会话存一份记录。这里只读定位需要的字段。
#[derive(Debug, Deserialize)]
struct DesktopSession {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "cliSessionId")]
    cli_session_id: Option<String>,
    cwd: Option<String>,
    #[serde(default, rename = "isArchived")]
    is_archived: bool,
    #[serde(default, rename = "lastActivityAt")]
    last_activity_at: i64,
}

fn desktop_sessions_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(DESKTOP_SESSIONS_DIR))
}

/// 找出这个 CLI 会话此刻属于哪个桌面窗口。
///
/// `cliSessionId` 不是唯一键：worktree 被删除后会话迁回主仓库、fork，或者
/// 一次 `claude://resume` 导入，都会让同一个 CLI 会话对应多条记录。工作目录
/// 能把真身和影子分开——Hook 报的 cwd 就是那个进程实际待的地方。
fn desktop_session_id(dir: &Path, cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
    let mut candidates = Vec::new();
    collect_desktop_sessions(dir, 0, &mut candidates);
    candidates.retain(|session| {
        !session.is_archived && session.cli_session_id.as_deref() == Some(cli_session_id)
    });
    if let Some(cwd) = cwd
        && let Some(exact) = candidates
            .iter()
            .find(|session| session.cwd.as_deref() == Some(cwd))
    {
        return Some(exact.session_id.clone());
    }
    candidates.sort_by_key(|session| session.last_activity_at);
    candidates.pop().map(|session| session.session_id)
}

/// 记录按 `<账号>/<组织>/local_*.json` 分层存放，层数不深，逐层读下去即可。
fn collect_desktop_sessions(dir: &Path, depth: u32, out: &mut Vec<DesktopSession>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_sessions(&path, depth + 1, out);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(session) = serde_json::from_str::<DesktopSession>(&text) {
                out.push(session);
            }
        }
    }
}

fn valid_desktop_id(value: &str) -> bool {
    value.starts_with(DESKTOP_ID_PREFIX) && valid_identifier(value)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(name), None) if valid(owner) && valid(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_thread_uses_the_native_deep_link() {
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
            },
            None,
        )
        .expect("UUID 应可打开");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CODEX_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "codex://threads/019c6e27-e55b-73d1-87d8-4e01f1f75043"
        );
    }

    #[test]
    fn a_known_window_is_focused_instead_of_reimported() {
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: Some("/work/agent-beacon".to_owned()),
            },
            Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"),
        )
        .expect("已知窗口应可打开");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CLAUDE_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "claude://code/continue?session=local_b65a60de-9adb-48b0-85c6-f9a178971322"
        );
    }

    #[test]
    fn a_cli_session_without_a_window_falls_back_to_importing() {
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: None,
            },
            None,
        )
        .expect("终端里的会话仍应可打开");

        assert_eq!(
            spec.args[2],
            "claude://resume?session=19b63622-e3e0-4cd0-a37e-dc8d71253155"
        );
    }

    #[test]
    fn a_forged_desktop_id_is_rejected() {
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                    cwd: None,
                },
                Some("local_../../tmp"),
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                    cwd: None,
                },
                Some("19b63622-e3e0-4cd0-a37e-dc8d71253155"),
            )
            .is_err(),
            "缺少 local_ 前缀的 id 不是桌面会话"
        );
    }

    #[test]
    fn malformed_remote_identifiers_are_rejected() {
        assert!(
            command_for(
                &ActivitySource::Codex {
                    thread_id: "../../tmp".to_owned(),
                },
                None,
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "../../tmp".to_owned(),
                    cwd: None,
                },
                None,
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::GitHubActions {
                    repo: "owner/repo/issues/1".to_owned(),
                    run_id: 1,
                },
                None,
            )
            .is_err()
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("agentbeacon-open-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    fn write_session(dir: &Path, id: &str, cli: &str, cwd: &str, archived: bool, activity: i64) {
        let record = format!(
            r#"{{"sessionId":"{id}","cliSessionId":"{cli}","cwd":"{cwd}","isArchived":{archived},"lastActivityAt":{activity}}}"#
        );
        std::fs::write(dir.join(format!("{id}.json")), record).expect("写入会话记录");
    }

    /// 一个 worktree 被删掉的会话会在索引里留下两条同 `cliSessionId` 的记录：
    /// 迁回主仓库的那条，和之前 K2 导入出来的影子。cwd 必须选中真身，否则 K2
    /// 打开的是一份内容停在昨天的陈旧会话。
    #[test]
    fn the_working_directory_picks_the_live_window() {
        let root = temp_dir("split");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        let cli = "19b63622-e3e0-4cd0-a37e-dc8d71253155";
        // 影子的活动时间更晚——导入本身就会刷新它，所以“取最新”是不够的。
        write_session(
            &dir,
            "local_19b63622-e3e0-4cd0-a37e-dc8d71253155",
            cli,
            "/work/agent-beacon",
            false,
            1_789_437_619_665,
        );
        write_session(
            &dir,
            "local_b65a60de-9adb-48b0-85c6-f9a178971322",
            cli,
            "/work/agent-beacon/.claude/worktrees/git-status",
            false,
            1_789_437_616_240,
        );

        let picked = desktop_session_id(
            &root,
            cli,
            Some("/work/agent-beacon/.claude/worktrees/git-status"),
        );
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            picked.as_deref(),
            Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"),
            "K2 应回到正在这个目录里干活的窗口"
        );
    }

    #[test]
    fn an_archived_window_is_never_reopened() {
        let root = temp_dir("archived");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        write_session(
            &dir,
            "local_aaaaaaaa-0000-0000-0000-000000000000",
            "cli-a",
            "/work/alpha",
            true,
            1,
        );

        let picked = desktop_session_id(&root, "cli-a", Some("/work/alpha"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(picked, None, "归档的会话不该被 K2 拉回来");
    }

    /// 同一个 CLI 会话只对应一个窗口时，cwd 对不上也不该退回导入。
    #[test]
    fn a_single_window_wins_even_when_the_directory_moved() {
        let root = temp_dir("moved");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        write_session(
            &dir,
            "local_cccccccc-0000-0000-0000-000000000000",
            "cli-c",
            "/work/new-place",
            false,
            5,
        );

        let picked = desktop_session_id(&root, "cli-c", Some("/work/old-place"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            picked.as_deref(),
            Some("local_cccccccc-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn github_run_uses_the_exact_run_url() {
        let spec = command_for(
            &ActivitySource::GitHubActions {
                repo: "longzhi/agent-beacon".to_owned(),
                run_id: 42,
            },
            None,
        )
        .expect("规范仓库名应可打开");

        assert_eq!(
            spec.args,
            ["https://github.com/longzhi/agent-beacon/actions/runs/42"]
        );
    }
}
