//! GitHub Actions 状态回流。
//!
//! CI 出结果的时候用户通常早就切走了，而这个盒子一直在视野边缘。这是设备
//! 真正比笔记本屏幕有用的场景，因此 CI 与 Agent 共用同一套任务卡和播报。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use beacon_protocol::Event;
use serde::Deserialize;
use tracing::{info, warn};

use crate::activity::{ActivityId, ActivityStatus, ActivityTracker, display_title};

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
}

impl CiWatcher {
    /// 取回各仓库最近一次 run。不持有聚合器的锁，因为 `gh` 可能要跑上几秒。
    pub async fn fetch(&mut self) -> Vec<(String, Run)> {
        let mut fetched = Vec::new();
        for repo in watched_repos() {
            match latest_run(&repo).await {
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
            return tracker.observe(&id, &title, ActivityStatus::Working);
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

/// 监听哪些仓库。每轮都重新读，加一个仓库不需要重启 daemon。
fn watched_repos() -> Vec<String> {
    if let Ok(value) = std::env::var("BEACON_CI_REPOS") {
        return value
            .split(',')
            .map(str::trim)
            .filter(|repo| !repo.is_empty())
            .map(str::to_owned)
            .collect();
    }
    let Some(path) = config_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/agentbeacon/ci-repos"))
}

async fn latest_run(repo: &str) -> Result<Option<Run>, String> {
    // LaunchAgent 的 PATH 很短，`gh` 常常不在里面，所以留一个显式覆盖。
    let program = std::env::var("BEACON_GH").unwrap_or_else(|_| "gh".to_owned());
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
    fn a_running_workflow_takes_a_card() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        let events = watcher.apply(
            &mut tracker,
            vec![("longzhi/agent-beacon".to_owned(), run(1, "in_progress", ""))],
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "task.start");
        assert_eq!(events[0].title.as_deref(), Some("CI:AGENT-BEACON"));
    }

    #[test]
    fn a_finished_run_we_never_saw_running_is_not_announced() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        let events = watcher.apply(
            &mut tracker,
            vec![(
                "longzhi/agent-beacon".to_owned(),
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
        let repo = "longzhi/agent-beacon".to_owned();

        watcher.apply(
            &mut tracker,
            vec![(repo.clone(), run(7, "in_progress", ""))],
        );
        let events = watcher.apply(&mut tracker, vec![(repo, run(7, "completed", "failure"))]);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "task.error");
        assert_eq!(events[0].title.as_deref(), Some("CI:AGENT-BEACON"));
    }

    #[test]
    fn a_cancelled_run_does_not_claim_failure() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();
        let repo = "longzhi/agent-beacon".to_owned();

        watcher.apply(
            &mut tracker,
            vec![(repo.clone(), run(9, "in_progress", ""))],
        );
        let events = watcher.apply(&mut tracker, vec![(repo, run(9, "completed", "cancelled"))]);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "agent.idle");
    }
}
