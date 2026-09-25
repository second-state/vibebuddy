//! Feeds GitHub Actions status back to the box.
//!
//! By the time CI finishes the user has usually moved on, while the box sits at the edge of view. This is
//! where the device beats the laptop screen, so CI shares the agents' task cards and announcements.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use vibebuddy_protocol::Event;
use serde::Deserialize;
use tracing::{info, warn};

use crate::activity::{ActivityId, ActivitySource, ActivityStatus, ActivityTracker, display_title};

/// Prefix on task cards that tells sources apart.
const PREFIX: &str = "CI:";
/// Title used when the repository name has no displayable characters.
const FALLBACK_TITLE: &str = "CI";
/// Poll interval. CI runs take minutes; 30 seconds is plenty and won't burn through the GitHub API quota.
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Upper bound for one `gh` call, so a stuck network can't hang the poll task forever.
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
    /// Per repository, the run currently in progress that has already been drawn on screen.
    running: HashMap<String, u64>,
    /// Repositories that already reported an error. A persisting error isn't logged again.
    quiet: HashSet<String>,
    /// Repositories that already got their one log line.
    announced: HashSet<String>,
}

impl CiWatcher {
    /// Fetches each repository's latest run. Doesn't hold the aggregator lock, since `gh` may take seconds.
    pub async fn fetch(&mut self, workspaces: &[PathBuf]) -> Vec<(String, Run)> {
        let repos = watched_repos(workspaces);
        if repos.is_empty() {
            return Vec::new();
        }
        let program = gh_program();
        let mut fetched = Vec::new();
        for repo in repos {
            // Log one line the first time a repository is watched, or "why isn't CI showing" can't be answered.
            if self.announced.insert(repo.clone()) {
                info!(%repo, "watching CI");
            }
            match latest_run(&program, &repo).await {
                Ok(Some(run)) => {
                    if self.quiet.remove(&repo) {
                        info!(%repo, "CI status recovered");
                    }
                    fetched.push((repo, run));
                }
                Ok(None) => {
                    self.quiet.remove(&repo);
                }
                Err(error) => {
                    // A persisting error logged every 30 seconds is thousands of lines a day. Log only the first.
                    if self.quiet.insert(repo.clone()) {
                        warn!(%repo, %error, "failed to read CI status");
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

        // Only report runs we actually watched start. Otherwise every daemon restart would re-announce
        // the repository's latest historical result.
        if self.running.remove(repo) != Some(run.database_id) {
            return None;
        }
        match run.conclusion.as_str() {
            "success" => tracker.finish(&id, &title),
            // Cancelled and skipped aren't task failures; just put the card away quietly.
            "cancelled" | "skipped" | "neutral" => tracker.discard(&id, "ALL QUIET"),
            _ => tracker.fail(&id, &title),
        }
    }
}

/// Which repositories to watch: the GitHub repositories the agents worked in recently.
///
/// No list for the user to maintain: the daemon already knows where you work, a fact computed while
/// resolving task-card titles. `VIBEBUDDY_CI_REPOS` overrides it, for repositories not checked out here.
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

/// Reads the GitHub repository name from the project root. Only reads `.git/config`; no network, no git.
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

/// Supports `https://`, `ssh://` and `git@host:` remotes.
fn parse_slug(url: &str) -> Option<String> {
    let rest = url.split_once("github.com")?.1;
    // The separator must follow the host name directly, or `github.com.example.org` would pass as GitHub.
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

/// Finds `gh`. launchd only puts four system directories on `PATH`, and `gh` usually isn't in them;
/// making the user edit a plist is exactly the step this feature wants to spare them.
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
        .map_err(|_| "gh timed out".to_owned())?
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    let runs: Vec<Run> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cannot parse gh output: {error}"))?;
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
            "the daemon must not re-announce the latest historical run on every restart"
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
