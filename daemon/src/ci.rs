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
/// How many recent runs to read per repository. One push often starts several workflows at once; reading only the
/// latest run lost the others, leaving their cards stuck as working and their failures unannounced.
const RECENT_RUNS: &str = "20";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    database_id: u64,
    /// A re-run keeps the run's id and bumps this, so one execution is the pair of both.
    #[serde(default)]
    attempt: u32,
    status: String,
    #[serde(default)]
    conclusion: String,
}

/// What one repository's recent runs looked like the last time we read them.
#[derive(Default)]
struct RepoRuns {
    /// Runs in progress that already hold a card.
    running: HashSet<u64>,
    /// Executions whose end has been handled (announced, or recorded as history). Never announced again.
    ended: HashSet<(u64, u32)>,
}

#[derive(Default)]
pub struct CiWatcher {
    repos: HashMap<String, RepoRuns>,
    /// Repositories that already reported an error. A persisting error isn't logged again.
    quiet: HashSet<String>,
    /// Repositories that already got their one log line.
    announced: HashSet<String>,
}

impl CiWatcher {
    /// Fetches each repository's recent runs. Doesn't hold the aggregator lock, since `gh` may take seconds.
    pub async fn fetch(&mut self, workspaces: &[PathBuf]) -> Vec<(String, Vec<Run>)> {
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
            match recent_runs(&program, &repo).await {
                Ok(runs) => {
                    if self.quiet.remove(&repo) {
                        info!(%repo, "CI status recovered");
                    }
                    fetched.push((repo, runs));
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
        fetched: Vec<(String, Vec<Run>)>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        for (repo, runs) in fetched {
            // The first read of a repository is its history: runs already over are recorded, never announced.
            // Otherwise every daemon restart would re-announce the repository's past results.
            let first_read = !self.repos.contains_key(&repo);
            let state = self.repos.entry(repo.clone()).or_default();
            let name = repo.rsplit('/').next().unwrap_or(&repo);
            let title = display_title(PREFIX, name, FALLBACK_TITLE);
            for run in &runs {
                let id = ActivityId {
                    session_id: format!("ci:{repo}"),
                    key: format!("ci:{repo}:{}", run.database_id),
                };
                if let Some(event) = Self::apply_one(state, tracker, &repo, &id, &title, run, first_read) {
                    events.push(event);
                }
            }
            // A run still marked running but gone from the list was pushed out by newer ones: put its card away.
            let listed: HashSet<u64> = runs.iter().map(|run| run.database_id).collect();
            let gone: Vec<u64> = state.running.iter().copied().filter(|id| !listed.contains(id)).collect();
            for gone in gone {
                state.running.remove(&gone);
                let id = ActivityId { session_id: format!("ci:{repo}"), key: format!("ci:{repo}:{gone}") };
                events.extend(tracker.discard(&id, "ALL QUIET"));
            }
            state.ended.retain(|(id, _)| listed.contains(id));
        }
        events
    }

    fn apply_one(
        state: &mut RepoRuns,
        tracker: &mut ActivityTracker,
        repo: &str,
        id: &ActivityId,
        title: &str,
        run: &Run,
        first_read: bool,
    ) -> Option<Event> {
        if run.status != "completed" {
            state.running.insert(run.database_id);
            let event = tracker.observe(id, title, ActivityStatus::Working);
            tracker.associate_source(
                id,
                ActivitySource::GitHubActions {
                    repo: repo.to_owned(),
                    run_id: run.database_id,
                },
            );
            return event;
        }

        // Each execution ends once. A re-run is a new execution: if it fails again, that is news.
        if !state.ended.insert((run.database_id, run.attempt)) || first_read {
            return None;
        }
        let tracked = state.running.remove(&run.database_id);
        let quiet = matches!(run.conclusion.as_str(), "cancelled" | "skipped" | "neutral");
        if !tracked {
            // Started and ended between two polls: nothing to put away if it was cancelled, otherwise still the
            // user's result, so give it a card for the moment it is announced.
            if quiet {
                return None;
            }
            tracker.observe(id, title, ActivityStatus::Working);
            tracker.associate_source(
                id,
                ActivitySource::GitHubActions {
                    repo: repo.to_owned(),
                    run_id: run.database_id,
                },
            );
        }
        match run.conclusion.as_str() {
            "success" => tracker.finish(id, title),
            // Cancelled and skipped aren't task failures; just put the card away quietly.
            _ if quiet => tracker.discard(id, "ALL QUIET"),
            _ => tracker.fail(id, title),
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

async fn recent_runs(program: &str, repo: &str) -> Result<Vec<Run>, String> {
    let command = tokio::process::Command::new(program)
        .args([
            "run",
            "list",
            "--repo",
            repo,
            "--limit",
            RECENT_RUNS,
            "--json",
            "databaseId,attempt,status,conclusion",
        ])
        .output();

    let output = tokio::time::timeout(COMMAND_TIMEOUT, command)
        .await
        .map_err(|_| "gh timed out".to_owned())?
        .map_err(|error| format!("cannot run gh: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    serde_json::from_slice(&output.stdout).map_err(|error| format!("cannot parse gh output: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "longzhi/vibe-buddy";

    fn run(id: u64, status: &str, conclusion: &str) -> Run {
        Run {
            database_id: id,
            attempt: 1,
            status: status.to_owned(),
            conclusion: conclusion.to_owned(),
        }
    }

    fn attempt(run: Run, attempt: u32) -> Run {
        Run { attempt, ..run }
    }

    fn poll(watcher: &mut CiWatcher, tracker: &mut ActivityTracker, runs: Vec<Run>) -> Vec<Event> {
        watcher.apply(tracker, vec![(REPO.to_owned(), runs)])
    }

    fn names(events: &[Event]) -> Vec<&str> {
        events.iter().map(|event| event.event.as_str()).collect()
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

        let events = poll(&mut watcher, &mut tracker, vec![run(1, "in_progress", "")]);

        assert_eq!(names(&events), ["task.start"]);
        assert_eq!(events[0].title.as_deref(), Some("CI:VIBE-BUDDY"));
    }

    #[test]
    fn runs_already_over_on_the_first_read_are_history() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        let events = poll(
            &mut watcher,
            &mut tracker,
            vec![run(2, "completed", "failure"), run(1, "completed", "success")],
        );

        assert!(
            events.is_empty(),
            "the daemon must not re-announce past results on every restart"
        );
    }

    #[test]
    fn a_failing_run_reports_an_error_once() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(7, "in_progress", "")]);
        let events = poll(&mut watcher, &mut tracker, vec![run(7, "completed", "failure")]);
        assert_eq!(names(&events), ["task.error"]);
        assert_eq!(events[0].title.as_deref(), Some("CI:VIBE-BUDDY"));

        let again = poll(&mut watcher, &mut tracker, vec![run(7, "completed", "failure")]);
        assert!(again.is_empty(), "the same failure must not be announced on every poll");
    }

    #[test]
    fn a_cancelled_run_does_not_claim_failure() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(9, "in_progress", "")]);
        let events = poll(&mut watcher, &mut tracker, vec![run(9, "completed", "cancelled")]);

        assert_eq!(names(&events), ["agent.idle"]);
    }

    /// One push starts several workflows. Reading only the latest run lost the earlier one: its card stayed
    /// working and its failure was never announced.
    #[test]
    fn parallel_workflows_each_report_their_own_end() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(11, "in_progress", ""), run(10, "in_progress", "")]);
        let events = poll(
            &mut watcher,
            &mut tracker,
            vec![run(11, "in_progress", ""), run(10, "completed", "failure")],
        );
        assert_eq!(names(&events), ["task.error"]);

        let events = poll(
            &mut watcher,
            &mut tracker,
            vec![run(11, "completed", "success"), run(10, "completed", "failure")],
        );
        assert_eq!(names(&events), ["task.done"]);
    }

    /// A re-run keeps the run id. Its failure is new information, but only once per attempt.
    #[test]
    fn a_rerun_that_fails_again_is_announced_once_more() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(7, "in_progress", "")]);
        poll(&mut watcher, &mut tracker, vec![run(7, "completed", "failure")]);
        poll(&mut watcher, &mut tracker, vec![attempt(run(7, "in_progress", ""), 2)]);
        let events = poll(&mut watcher, &mut tracker, vec![attempt(run(7, "completed", "failure"), 2)]);
        assert_eq!(names(&events), ["task.error"]);

        let again = poll(&mut watcher, &mut tracker, vec![attempt(run(7, "completed", "failure"), 2)]);
        assert!(again.is_empty());
    }

    /// A run that starts and ends between two polls was never seen running, yet it is still the user's result.
    #[test]
    fn a_run_shorter_than_the_poll_interval_is_still_announced() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(1, "completed", "success")]);
        let events = poll(
            &mut watcher,
            &mut tracker,
            vec![run(2, "completed", "failure"), run(1, "completed", "success")],
        );
        assert_eq!(names(&events), ["task.error"]);

        let rerun = poll(
            &mut watcher,
            &mut tracker,
            vec![run(2, "completed", "failure"), attempt(run(1, "completed", "failure"), 2)],
        );
        assert_eq!(names(&rerun), ["task.error"], "a quick re-run is a new attempt");
    }

    #[test]
    fn a_quick_cancelled_run_stays_silent() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![]);
        let events = poll(&mut watcher, &mut tracker, vec![run(3, "completed", "cancelled")]);
        assert!(events.is_empty());
    }

    #[test]
    fn a_running_card_pushed_out_of_the_list_is_put_away() {
        let mut watcher = CiWatcher::default();
        let mut tracker = ActivityTracker::default();

        poll(&mut watcher, &mut tracker, vec![run(1, "in_progress", "")]);
        let events = poll(&mut watcher, &mut tracker, vec![run(2, "in_progress", "")]);

        let tasks = events.last().and_then(|event| event.extra.get("tasks")).and_then(|tasks| tasks.as_array());
        assert_eq!(tasks.map(Vec::len), Some(1), "the card for run 1 must not stay forever: {events:?}");
    }
}
