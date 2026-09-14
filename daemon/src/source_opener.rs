//! 把聚合活动定位回 Mac 上的来源窗口。
//!
//! 所有参数都直接交给进程 API，不经过 shell。来源数据来自 Hook，仍按不可信
//! 输入处理：Codex thread id、Claude session id 与 GitHub repo 都先收窄字符集。

use std::time::Duration;

use tokio::process::Command;

use crate::activity::ActivitySource;

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const CODEX_BUNDLE_ID: &str = "com.openai.codex";
const CLAUDE_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

#[derive(Debug, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

pub async fn open(source: ActivitySource) -> Result<(), String> {
    let spec = command_for(&source)?;
    let output = tokio::time::timeout(
        OPEN_TIMEOUT,
        Command::new(spec.program).args(&spec.args).output(),
    )
    .await
    .map_err(|_| "打开来源超时".to_owned())?
    .map_err(|error| format!("无法启动打开命令：{error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("打开来源失败：{}", stderr.trim()))
}

fn command_for(source: &ActivitySource) -> Result<CommandSpec, String> {
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
        ActivitySource::ClaudeCode { session_id } => {
            if !valid_identifier(session_id) {
                return Err("Claude session id 含有非法字符".to_owned());
            }
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec![
                    "-b".to_owned(),
                    CLAUDE_BUNDLE_ID.to_owned(),
                    format!("claude://resume?session={session_id}"),
                ],
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
        let spec = command_for(&ActivitySource::Codex {
            thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
        })
        .expect("UUID 应可打开");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CODEX_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "codex://threads/019c6e27-e55b-73d1-87d8-4e01f1f75043"
        );
    }

    #[test]
    fn claude_session_uses_the_native_resume_deep_link() {
        let spec = command_for(&ActivitySource::ClaudeCode {
            session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
        })
        .expect("Claude Code UUID 应可打开");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CLAUDE_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "claude://resume?session=19b63622-e3e0-4cd0-a37e-dc8d71253155"
        );
    }

    #[test]
    fn malformed_remote_identifiers_are_rejected() {
        assert!(
            command_for(&ActivitySource::Codex {
                thread_id: "../../tmp".to_owned(),
            })
            .is_err()
        );
        assert!(
            command_for(&ActivitySource::ClaudeCode {
                session_id: "../../tmp".to_owned(),
            })
            .is_err()
        );
        assert!(
            command_for(&ActivitySource::GitHubActions {
                repo: "owner/repo/issues/1".to_owned(),
                run_id: 1,
            })
            .is_err()
        );
    }

    #[test]
    fn github_run_uses_the_exact_run_url() {
        let spec = command_for(&ActivitySource::GitHubActions {
            repo: "longzhi/agent-beacon".to_owned(),
            run_id: 42,
        })
        .expect("规范仓库名应可打开");

        assert_eq!(
            spec.args,
            ["https://github.com/longzhi/agent-beacon/actions/runs/42"]
        );
    }
}
