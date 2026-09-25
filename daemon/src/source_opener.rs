//! Navigates from an aggregated activity back to its source window on the Mac.
//!
//! All arguments go straight to the process API, never through a shell. Source data comes from hooks and is still treated
//! as untrusted input: Codex thread ids, Claude session ids and GitHub repos are narrowed to a safe character set first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

use crate::activity::{ActivitySource, Surface};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const CODEX_BUNDLE_ID: &str = "com.openai.codex";
const CLAUDE_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
/// Where Claude App keeps a record of each Code session, one subdirectory per account.
const DESKTOP_SESSIONS_DIR: &str = "Library/Application Support/Claude/claude-code-sessions";
/// Prefix of desktop session ids; the deep link only accepts this form.
const DESKTOP_ID_PREFIX: &str = "local_";

#[derive(Debug, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

/// On success returns the link actually opened, so the log can say where K2 went.
pub async fn open(source: ActivitySource) -> Result<String, String> {
    let desktop = reported_desktop_session(&source)
        .map(str::to_owned)
        .or_else(|| fallback_desktop_session(&source));
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
    // First answer one question: is the user inside this agent's app? If not, don't use the deeplink;
    // it would import a terminal session into the app as a copy.
    match surface_of(source) {
        Some(Surface::Host { bundle_id }) => return activate(bundle_id),
        Some(Surface::Headless) => return Err("会话没有宿主窗口（SSH 或后台进程）".to_owned()),
        _ => {}
    }
    match source {
        ActivitySource::Codex { thread_id, .. } => {
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
            // When we already know the window, focus it directly. `claude://resume` takes a different
            // route: it claims a transcript on disk by CLI session id. When the same id has a copy in several
            // project directories it can only pick one, and picking wrong opens a stale
            // shadow session, re-importing the whole transcript on every press.
            let link = match desktop {
                Some(desktop) => {
                    if !valid_desktop_id(desktop) {
                        return Err("桌面会话 id 格式无效".to_owned());
                    }
                    format!("claude://code/continue?session={desktop}")
                }
                // CLI sessions running in a terminal have no matching window; importing is the only way to open them.
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

/// Code sessions started by Claude App put the desktop session id in the process environment, and the hook reports it verbatim.
/// It's an identity the app itself assigned and can be used directly, with no need to claim a
/// transcript on disk by CLI session id, which is one-to-many and opens a stale shadow session (see LESSONS.md).
fn reported_desktop_session(source: &ActivitySource) -> Option<&str> {
    match source {
        ActivitySource::ClaudeCode { surface: Surface::App { desktop_session_id }, .. } => desktop_session_id.as_deref(),
        _ => None,
    }
}

/// Old hooks and old state files don't report the desktop session id, so disambiguate by cwd in the session index.
fn fallback_desktop_session(source: &ActivitySource) -> Option<String> {
    let ActivitySource::ClaudeCode { session_id, cwd, surface: Surface::App { desktop_session_id: None } } = source else {
        return None;
    };
    desktop_session_by_cli(&desktop_sessions_dir()?, session_id, cwd.as_deref())
}

fn surface_of(source: &ActivitySource) -> Option<&Surface> {
    match source {
        ActivitySource::Codex { surface, .. } | ActivitySource::ClaudeCode { surface, .. } => Some(surface),
        ActivitySource::GitHubActions { .. } => None,
    }
}

/// Brings the host app to the front. Whether we recognise the bundle id doesn't matter: unfamiliar terminals take
/// this same path, so supporting a new terminal needs no code change. The value comes from the hook and is still
/// narrowed to a safe character set as untrusted input.
fn activate(bundle_id: &str) -> Result<CommandSpec, String> {
    if !valid_bundle_id(bundle_id) {
        return Err("宿主 bundle id 格式无效".to_owned());
    }
    Ok(CommandSpec {
        program: "/usr/bin/open",
        args: vec!["-b".to_owned(), bundle_id.to_owned()],
    })
}

/// Claude App stores one record per Code session. Only the fields needed for navigation are read here.
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
    /// The session title the app generated; the task card's first line uses it.
    #[serde(default)]
    title: Option<String>,
}

fn desktop_sessions_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(DESKTOP_SESSIONS_DIR))
}

/// Finds which desktop window this CLI session belongs to right now.
///
/// `cliSessionId` isn't a unique key: a session moving back to the main repository after its worktree is deleted, a fork, or
/// a `claude://resume` import all give one CLI session several records. The working directory
/// tells the real one from the shadows: the cwd the hook reports is where that process actually is.
fn desktop_session_by_cli(dir: &Path, cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
    desktop_session(dir, cli_session_id, cwd).map(|session| session.session_id)
}

/// This CLI session's title in Claude App. Records are picked by the same rule as when opening the window.
pub(crate) fn desktop_session_title(cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
    let dir = desktop_sessions_dir()?;
    desktop_session(&dir, cli_session_id, cwd)
        .and_then(|session| session.title)
        .filter(|title| !title.trim().is_empty())
}

fn desktop_session(dir: &Path, cli_session_id: &str, cwd: Option<&str>) -> Option<DesktopSession> {
    let mut candidates = Vec::new();
    collect_desktop_sessions(dir, 0, &mut candidates);
    candidates.retain(|session| {
        !session.is_archived && session.cli_session_id.as_deref() == Some(cli_session_id)
    });
    if let Some(cwd) = cwd
        && let Some(index) = candidates
            .iter()
            .position(|session| session.cwd.as_deref() == Some(cwd))
    {
        return Some(candidates.swap_remove(index));
    }
    candidates.sort_by_key(|session| session.last_activity_at);
    candidates.pop()
}

/// Records are stored as `<account>/<org>/local_*.json`; the tree is shallow, so read it level by level.
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

/// Bundle ids allow the same characters as session ids, but must be dotted, with no paths or whitespace.
fn valid_bundle_id(value: &str) -> bool {
    valid_identifier(value) && value.contains('.')
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
    fn a_reported_desktop_session_skips_the_disk_lookup() {
        // The app's own identity wins; no more claiming transcripts on disk by CLI session id.
        let source = ActivitySource::ClaudeCode {
            session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            surface: Surface::App {
                desktop_session_id: Some("local_b65a60de-9adb-48b0-85c6-f9a178971322".to_owned()),
            },
        };

        assert_eq!(reported_desktop_session(&source), Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"));
        assert!(fallback_desktop_session(&source).is_none());
    }

    #[test]
    fn a_terminal_session_never_looks_for_a_desktop_window() {
        let source = ActivitySource::ClaudeCode {
            session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            surface: Surface::Host { bundle_id: "com.mitchellh.ghostty".to_owned() },
        };

        assert!(reported_desktop_session(&source).is_none());
        assert!(fallback_desktop_session(&source).is_none());
    }

    #[test]
    fn a_terminal_session_activates_its_host_instead_of_importing() {
        // The user runs claude in Ghostty: the deeplink would import the session into the app as a copy;
        // the right move is to bring that terminal to the front.
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: Some("/work/vibe-buddy".to_owned()),
                surface: Surface::Host { bundle_id: "com.mitchellh.ghostty".to_owned() },
            },
            None,
        )
        .expect("宿主应可激活");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args, ["-b", "com.mitchellh.ghostty"]);
    }

    #[test]
    fn an_unknown_host_needs_no_code_change() {
        // Unfamiliar terminals take the same path as known ones, so switching terminals needs no code change.
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::Host { bundle_id: "net.example.SomeNewTerminal".to_owned() },
            },
            None,
        )
        .expect("未知宿主也该激活");

        assert_eq!(spec.args, ["-b", "net.example.SomeNewTerminal"]);
    }

    #[test]
    fn a_headless_session_is_skipped_rather_than_opened_wrong() {
        // Sessions started over SSH or by a daemon have no window at all; return an error so K2 tries the next candidate.
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::Headless,
            },
            None,
        );

        assert!(spec.is_err());
    }

    #[test]
    fn a_host_id_that_is_not_a_bundle_id_is_refused() {
        // The bundle id comes from the hook and is treated as untrusted input.
        for bogus in ["../../evil", "com.example.a b", "no-dots", ""] {
            assert!(
                command_for(
                    &ActivitySource::Codex {
                        thread_id: "t".to_owned(),
                        surface: Surface::Host { bundle_id: bogus.to_owned() },
                    },
                    None,
                )
                .is_err(),
                "{bogus} 不该被接受"
            );
        }
    }

    #[test]
    fn a_host_without_a_bundle_id_has_nowhere_to_go() {
        // The hook says there's a host but gave no target: skip it, rather than fall back to importing the session into the app.
        assert_eq!(Surface::from_hook(Some("host"), None, None), Surface::Headless);
    }

    #[test]
    fn an_older_hook_keeps_the_desktop_behaviour() {
        // Old hooks and old state files report no surface; back then only the desktop app was supported.
        assert_eq!(Surface::from_hook(None, None, None), Surface::default());
        assert!(matches!(Surface::default(), Surface::App { .. }));
    }

    #[test]
    fn codex_thread_uses_the_native_deep_link() {
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::default(),
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
                cwd: Some("/work/vibe-buddy".to_owned()),
                surface: Surface::default(),
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
                surface: Surface::default(),
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
                    surface: Surface::default(),
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
                    surface: Surface::default(),
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
                    surface: Surface::default(),
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
                    surface: Surface::default(),
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
            std::env::temp_dir().join(format!("vibebuddy-open-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    fn write_session(dir: &Path, id: &str, cli: &str, cwd: &str, archived: bool, activity: i64) {
        let record = format!(
            r#"{{"sessionId":"{id}","cliSessionId":"{cli}","cwd":"{cwd}","isArchived":{archived},"lastActivityAt":{activity}}}"#
        );
        std::fs::write(dir.join(format!("{id}.json")), record).expect("写入会话记录");
    }

    /// A session whose worktree was deleted leaves two records with the same `cliSessionId` in the index:
    /// the one moved back to the main repository, and the shadow an earlier K2 imported. cwd must pick the real one, or K2
    /// opens a stale session whose content stopped yesterday.
    #[test]
    fn the_working_directory_picks_the_live_window() {
        let root = temp_dir("split");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        let cli = "19b63622-e3e0-4cd0-a37e-dc8d71253155";
        // The shadow has the later activity time, since importing itself refreshes it, so "take the newest" isn't enough.
        write_session(
            &dir,
            "local_19b63622-e3e0-4cd0-a37e-dc8d71253155",
            cli,
            "/work/vibe-buddy",
            false,
            1_789_437_619_665,
        );
        write_session(
            &dir,
            "local_b65a60de-9adb-48b0-85c6-f9a178971322",
            cli,
            "/work/vibe-buddy/.claude/worktrees/git-status",
            false,
            1_789_437_616_240,
        );

        let picked = desktop_session_by_cli(
            &root,
            cli,
            Some("/work/vibe-buddy/.claude/worktrees/git-status"),
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

        let picked = desktop_session_by_cli(&root, "cli-a", Some("/work/alpha"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(picked, None, "归档的会话不该被 K2 拉回来");
    }

    /// When a CLI session maps to a single window, a cwd mismatch still mustn't fall back to importing.
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

        let picked = desktop_session_by_cli(&root, "cli-c", Some("/work/old-place"));
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
                repo: "longzhi/vibe-buddy".to_owned(),
                run_id: 42,
            },
            None,
        )
        .expect("规范仓库名应可打开");

        assert_eq!(
            spec.args,
            ["https://github.com/longzhi/vibe-buddy/actions/runs/42"]
        );
    }
}
