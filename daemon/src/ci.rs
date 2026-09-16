//! GitHub Actions 状态回流。
//!
//! CI 出结果的时候用户通常早就切走了，而这个盒子一直在视野边缘。这是设备
//! 真正比笔记本屏幕有用的场景，因此 CI 与 Agent 共用同一套任务卡和播报。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use vibebuddy_protocol::Event;
use serde::Deserialize;
use tracing::{info, warn};

use crate::activity::{ActivityId, ActivitySource, ActivityStatus, ActivityTracker, display_title};

/// 任务卡上区分来源的前缀。
const PREFIX: &str = "CI:";
/// 仓库名里没有可显示字符时的标题。
const FALLBACK_TITLE: &str = "CI";
/// 轮询间隔。CI 以分钟计，30 秒足够，也不至于把 GitHub API 配额用光。
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// 单次 `gh` 调用的上限，避免网络卡住时把轮询任务永久挂起。
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    database_id: u64,
    status: String,
    #[serde(default)]
    conclusion: String,
}

#[derive(Default)]
pub struct CiWatcher {
    /// 每个仓库当前正在跑、且已经被画上屏幕的 run。
    running: HashMap<String, u64>,
    /// 已经报过错的仓库。错误持续存在时不再重复刷日志。
    quiet: HashSet<String>,
    /// 已经记过一行日志的仓库。
    announced: HashSet<String>,
}

impl CiWatcher {
    /// 取回各仓库最近一次 run。不持有聚合器的锁，因为 `gh` 可能要跑上几秒。
    pub async fn fetch(&mut self, workspaces: &[PathBuf]) -> Vec<(String, Run)> {
        let repos = watched_repos(workspaces);
        if repos.is_empty() {
            return Vec::new();
        }
        let program = gh_program();
        let mut fetched = Vec::new();
        for repo in repos {
            // 第一次关注某个仓库时记一行，否则「CI 怎么没显示」无从查起。
            if self.announced.insert(repo.clone()) {
                info!(%repo, "开始关注 CI");
            }
            match latest_run(&program, &repo).await {
                Ok(Some(run)) => {
                    if self.quiet.remove(&repo) {
                        info!(%repo, "CI 状态已恢复");
                    }
                    fetched.push((repo, run));
                }
                Ok(None) => {
                    self.quiet.remove(&repo);
                }
                Err(error) => {
                    // 一个持续的错误每 30 秒记一行，一天就是几千行。只记第一次。
                    if self.quiet.insert(repo.clone()) {
                        warn!(%repo, %error, "读取 CI 状态失败");
                    }
                }
            }
        }
        fetched
    }

    pub fn apply(
        &mut self,
        tracker: &mut ActivityTracker,
        fetched: Vec<(String, Run)>,
    ) -> Vec<Event> {
        fetched
            .into_iter()
            .filter_map(|(repo, run)| self.apply_one(tracker, &repo, &run))
            .collect()
    }

    fn apply_one(&mut self, tracker: &mut ActivityTracker, repo: &str, run: &Run) -> Option<Event> {
        let name = repo.rsplit('/').next().unwrap_or(repo);
        let title = display_title(PREFIX, name, FALLBACK_TITLE);
        let id = ActivityId {
            session_id: format!("ci:{repo}"),
            key: format!("ci:{repo}:{}", run.database_id),
        };

        if run.status != "completed" {
            self.running.insert(repo.to_owned(), run.database_id);
            let event = tracker.observe(&id, &title, ActivityStatus::Working);
            tracker.associate_source(
                &id,
                ActivitySource::GitHubActions {
                    repo: repo.to_owned(),
                    run_id: run.database_id,
                },
            );
            return event;
        }

        // 只报告亲眼看着跑起来的 run。否则 daemon 每次重启都会把仓库里
        // 最近一次历史结果重新播报一遍。
        if self.running.remove(repo) != Some(run.database_id) {
            return None;
        }
        match run.conclusion.as_str() {
            "success" => tracker.finish(&id, &title),
            // 取消和跳过都不是任务失败，安静收起卡片即可。
            "cancelled" | "skipped" | "neutral" => tracker.discard(&id, "ALL QUIET"),
            _ => tracker.fail(&id, &title),
        }
    }
}

/// 关注哪些仓库：Agent 最近工作过的那些 GitHub 仓库。
///
/// 不需要用户维护清单——daemon 已经知道你在哪儿干活，这个事实在解析任务卡
/// 标题时就算出来了。`VIBEBUDDY_CI_REPOS` 可以覆盖，用于观察本机没有检出的仓库。
fn watched_repos(workspaces: &[PathBuf]) -> Vec<String> {
    if let Ok(value) = std::env::var("VIBEBUDDY_CI_REPOS") {
        return value
            .split(',')
            .map(str::trim)
            .filter(|repo| !repo.is_empty())
            .map(str::to_owned)
            .collect();
    }
    let mut repos: Vec<String> = workspaces
        .iter()
        .filter_map(|root| github_slug(root))
        .collect();
    repos.sort();
    repos.dedup();
    repos
}

/// 从项目根读出 GitHub 仓库名。只读 `.git/config`，不调用网络也不调用 git。
fn github_slug(root: &Path) -> Option<String> {
    let config = std::fs::read_to_string(root.join(".git").join("config")).ok()?;
    parse_slug(&origin_url(&config)?)
}

fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.starts_with("[remote \"origin\"]");
        } else if in_origin && let Some(value) = line.strip_prefix("url") {
            return Some(value.trim_start().strip_prefix('=')?.trim().to_owned());
        }
    }
    None
}

/// 支持 `https://`、`ssh://` 和 `git@host:` 三种远端写法。
fn parse_slug(url: &str) -> Option<String> {
    let rest = url.split_once("github.com")?.1;
    // 分隔符必须紧跟在主机名之后，否则 `github.com.example.org` 也会被认成 GitHub。
    if !rest.starts_with(':') && !rest.starts_with('/') {
        return None;
    }
    let rest = rest[1..].trim_start_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("{owner}/{repo}"))
}

/// 找到 `gh`。launchd 只给四个系统目录的 `PATH`，`gh` 通常不在里面；
/// 而让用户去改 plist 正是这个功能想省掉的那一步。
fn gh_program() -> String {
    if let Ok(path) = std::env::var("VIBEBUDDY_GH") {
        return path;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    [
        format!("{home}/bin/gh"),
        "/opt/homebrew/bin/gh".to_owned(),
        "/usr/local/bin/gh".to_owned(),
        "/usr/bin/gh".to_owned(),
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .unwrap_or_else(|| "gh".to_owned())
}

async fn latest_run(program: &str, repo: &str) -> Result<Option<Run>, String> {
    let command = tokio::process::Command::new(program)
        .args([
            "run",
            "list",
            "--repo",
            repo,
            "--limit",
            "1",
            "--json",
            "databaseId,status,conclusion",
        ])
        .output();

    let output = tokio::time::timeout(COMMAND_TIMEOUT, command)
        .await
        .map_err(|_| "gh 超时".to_owned())?
        .map_err(|error| format!("无法执行 gh：{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    let runs: Vec<Run> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("gh 输出无法解析：{error}"))?;
    Ok(runs.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: u64, status: &str, conclusion: &str) -> Run {
        Run {
            database_id: id,
            status: status.to_owned(),
            conclusion: conclusion.to_owned(),
        }
    }

    #[test]
    fn every_common_remote_spelling_resolves_to_the_same_repository() {
        for url in [
            "https://github.com/longzhi/vibe-buddy.git",
            "git@github.com:longzhi/vibe-buddy.git",
            "ssh://git@github.com/longzhi/vibe-buddy",
        ] {
            assert_eq!(
                parse_slug(url).as_deref(),
                Some("longzhi/vibe-buddy"),
                "{url}"
            );
        }
    }

    #[test]
    fn a_lookalike_host_is_not_github() {
        assert_eq!(parse_slug("https://github.com.example.org/a/b"), None);
        assert_eq!(parse_slug("https://gitlab.com/a/b.git"), None);
    }

    #[test]
    fn the_origin_remote_wins_over_the_others() {
        let config = concat!(
            "[remote \"upstream\"]\n",
            "\turl = https://github.com/someone/fork.git\n",
            "[remote \"origin\"]\n",
            "\turl = https://github.com/longzhi/vibe-buddy.git\n",
            "[branch \"main\"]\n",
            "\tremote = origin\n",
        );
        assert_eq!(
            origin_url(config).as_deref(),
            Some("https://github.com/longzhi/vibe-buddy.git")
        );
    }

    #[test]
    fn a_running_workflow_takes_a_card() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        let events = watcher.apply(
            &mut tracker,
            vec![("longzhi/vibe-buddy".to_owned(), run(1, "in_progress", ""))],
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "task.start");
        assert_eq!(events[0].title.as_deref(), Some("CI:VIBE-BUDDY"));
    }

    #[test]
    fn a_finished_run_we_never_saw_running_is_not_announced() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        let events = watcher.apply(
            &mut tracker,
            vec![(
                "longzhi/vibe-buddy".to_owned(),
                run(1, "completed", "success"),
            )],
        );

        assert!(
            events.is_empty(),
            "daemon 每次重启都把仓库最近一次历史结果播一遍是不能接受的"
        );
    }

    #[test]
    fn a_failing_run_reports_an_error() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();
        let repo = "longzhi/vibe-buddy".to_owned();

        watcher.apply(
            &mut tracker,
            vec![(repo.clone(), run(7, "in_progress", ""))],
        );
        let events = watcher.apply(&mut tracker, vec![(repo, run(7, "completed", "failure"))]);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "task.error");
        assert_eq!(events[0].title.as_deref(), Some("CI:VIBE-BUDDY"));
    }

    #[test]
    fn a_cancelled_run_does_not_claim_failure() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();
        let repo = "longzhi/vibe-buddy".to_owned();

        watcher.apply(
            &mut tracker,
            vec![(repo.clone(), run(9, "in_progress", ""))],
        );
        let events = watcher.apply(&mut tracker, vec![(repo, run(9, "completed", "cancelled"))]);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "agent.idle");
    }
}
