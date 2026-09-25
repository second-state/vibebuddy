//! Where a task card's first line comes from: the name the agent itself gave the session.
//!
//! The project name can't tell apart several things going on in one project. Both agents store session names locally:
//! Claude App's session index has an auto-generated `title`, and Codex's `state_5.sqlite`
//! has the user-given `name` and the thread's recorded `git_branch`. None of these is a prompt: the former is
//! the summary the app's sidebar already shows, the latter are names the user typed. Codex's `title`
//! column is the raw first message (mixed with injected context), so it's not used.
//!
//! Lookups are cached: hooks arrive every few seconds, so we can't scan directories and open databases each time. A title may
//! only be generated a while after the session starts, so misses are retried every 30 seconds and hits refreshed every 5 minutes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::OpenFlags;

use crate::source_opener;

const RETRY_UNRESOLVED: Duration = Duration::from_secs(30);
const REFRESH_RESOLVED: Duration = Duration::from_secs(300);
const CODEX_STATE_DB: &str = ".codex/state_5.sqlite";

pub struct SessionTitles {
    enabled: bool,
    codex_db: Option<PathBuf>,
    cache: HashMap<String, Cached>,
    /// Whether the thread exists in Codex's thread table; misses are cached too, so background sessions don't
    /// open the database on every hook.
    known: HashMap<String, (Option<bool>, Instant)>,
}

struct Cached {
    title: Option<String>,
    checked_at: Instant,
}

impl SessionTitles {
    pub fn from_home() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self {
            enabled: true,
            codex_db: home.map(|home| home.join(CODEX_STATE_DB)),
            cache: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// For tests: finds nothing, so titles fall back to branch or project name and every thread counts as existing.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            codex_db: None,
            cache: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// For tests: points at a specific Codex state database.
    #[cfg(test)]
    pub fn with_codex_db(db: PathBuf) -> Self {
        Self {
            enabled: true,
            codex_db: Some(db),
            cache: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// Whether this thread exists in Codex's thread table. Returns None when it can't tell (no database, can't open it);
    /// callers treat that as "exists": better to show one background session too many than to filter out a real one.
    pub fn codex_thread_known(&mut self, thread_id: &str) -> Option<bool> {
        if !self.enabled {
            return None;
        }
        let db = self.codex_db.clone()?;
        let now = Instant::now();
        if let Some((known, checked_at)) = self.known.get(thread_id) {
            let ttl = if *known == Some(true) {
                REFRESH_RESOLVED
            } else {
                RETRY_UNRESOLVED
            };
            if now.duration_since(*checked_at) < ttl {
                return *known;
            }
        }
        let known = codex_thread_exists(&db, thread_id);
        self.known.insert(thread_id.to_owned(), (known, now));
        known
    }

    /// The title Claude App gave this session. Sessions run directly in a terminal have no record.
    pub fn claude(&mut self, cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let key = format!("claude:{cli_session_id}");
        self.lookup(&key, || {
            source_opener::desktop_session_title(cli_session_id, cwd)
        })
    }

    /// The Codex thread's name: the user-given `name` first, then the thread's recorded branch.
    pub fn codex(&mut self, thread_id: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let db = self.codex_db.clone()?;
        let key = format!("codex:{thread_id}");
        self.lookup(&key, || codex_thread_title(&db, thread_id))
    }

    fn lookup(&mut self, key: &str, fetch: impl FnOnce() -> Option<String>) -> Option<String> {
        let now = Instant::now();
        if let Some(cached) = self.cache.get(key) {
            let ttl = if cached.title.is_some() {
                REFRESH_RESOLVED
            } else {
                RETRY_UNRESOLVED
            };
            if now.duration_since(cached.checked_at) < ttl {
                return cached.title.clone();
            }
        }
        let title = fetch();
        self.cache.insert(
            key.to_owned(),
            Cached {
                title: title.clone(),
                checked_at: now,
            },
        );
        title
    }
}

/// Opens Codex's state database read-only. Codex writes it itself (WAL); a read-only connection doesn't get in the way.
/// Any failure counts as no title.
pub fn codex_thread_title(db: &Path, thread_id: &str) -> Option<String> {
    let connection = rusqlite::Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let (name, branch): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT name, git_branch FROM threads WHERE id = ?1",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok()?;
    name.filter(|name| !name.trim().is_empty())
        .or_else(|| branch.filter(|branch| is_feature_branch(branch)))
        .map(|value| branch_tail(&value))
}

/// Whether the thread table has this id. Returns None when the database can't be opened.
pub fn codex_thread_exists(db: &Path, thread_id: &str) -> Option<bool> {
    let connection = rusqlite::Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    connection
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id = ?1",
            [thread_id],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .map(|count| count > 0)
}

/// The branch of the working directory; the main branch says nothing about the task, so it doesn't count.
///
/// A worktree's `.git` is a file pointing at that worktree's own directory inside the main repository, and the
/// `HEAD` there is its branch; going back to the main repository via `project_root` would read the wrong one.
pub fn git_branch(cwd: &str) -> Option<String> {
    for dir in Path::new(cwd).ancestors() {
        let git = dir.join(".git");
        let head = if git.is_dir() {
            git.join("HEAD")
        } else if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            PathBuf::from(text.strip_prefix("gitdir:")?.trim()).join("HEAD")
        } else {
            continue;
        };
        let head = std::fs::read_to_string(head).ok()?;
        let branch = head.trim().strip_prefix("ref: refs/heads/")?;
        return is_feature_branch(branch).then(|| branch_tail(branch));
    }
    None
}

fn is_feature_branch(branch: &str) -> bool {
    !matches!(branch.trim(), "" | "main" | "master" | "HEAD")
}

/// Keep only the last segment of `claude/pomodoro-timer-feature`: the prefix says who opened the branch,
/// which doesn't fit on the card and isn't needed.
fn branch_tail(branch: &str) -> String {
    branch
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or(branch)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "session-titles-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("创建临时目录");
        base
    }

    #[test]
    fn branch_comes_from_head_and_skips_main() {
        let repo = temp_dir("branch");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(git_branch(repo.to_str().unwrap()), None);

        std::fs::write(
            repo.join(".git/HEAD"),
            "ref: refs/heads/claude/pomodoro-timer-feature-b509bd\n",
        )
        .unwrap();
        let nested = repo.join("firmware/main");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            git_branch(nested.to_str().unwrap()).as_deref(),
            Some("pomodoro-timer-feature-b509bd")
        );
    }

    #[test]
    fn worktree_reads_its_own_head() {
        let main = temp_dir("wt-main");
        let worktree_git = main.join(".git/worktrees/feature");
        std::fs::create_dir_all(&worktree_git).unwrap();
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(worktree_git.join("HEAD"), "ref: refs/heads/feature-x\n").unwrap();
        let worktree = temp_dir("wt-tree");
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();
        assert_eq!(
            git_branch(worktree.to_str().unwrap()).as_deref(),
            Some("feature-x")
        );
    }

    #[test]
    fn codex_prefers_the_user_name_then_the_branch() {
        let dir = temp_dir("codex");
        let db = dir.join("state.sqlite");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, git_branch TEXT);
                 INSERT INTO threads VALUES ('named', 'Review 两个PR', 'feat/x');
                 INSERT INTO threads VALUES ('branched', '', 'codex/k2-source-navigation');
                 INSERT INTO threads VALUES ('bare', NULL, 'main');",
            )
            .unwrap();
        drop(connection);
        assert_eq!(codex_thread_title(&db, "named").as_deref(), Some("Review 两个PR"));
        assert_eq!(
            codex_thread_title(&db, "branched").as_deref(),
            Some("k2-source-navigation")
        );
        assert_eq!(codex_thread_title(&db, "bare"), None);
        assert_eq!(codex_thread_title(&db, "missing"), None);
        assert_eq!(codex_thread_title(&dir.join("absent.sqlite"), "named"), None);
    }

    #[test]
    fn thread_existence_is_checked_against_the_thread_table() {
        let dir = temp_dir("known");
        let db = dir.join("state.sqlite");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, git_branch TEXT);
                 INSERT INTO threads VALUES ('real', NULL, NULL);",
            )
            .unwrap();
        drop(connection);
        assert_eq!(codex_thread_exists(&db, "real"), Some(true));
        assert_eq!(codex_thread_exists(&db, "ghost"), Some(false));
        assert_eq!(codex_thread_exists(&dir.join("absent.sqlite"), "real"), None);

        let mut titles = SessionTitles::with_codex_db(db);
        assert_eq!(titles.codex_thread_known("real"), Some(true));
        assert_eq!(titles.codex_thread_known("ghost"), Some(false));
        assert_eq!(SessionTitles::disabled().codex_thread_known("ghost"), None);
    }

    #[test]
    fn lookups_are_cached_between_hook_events() {
        let mut titles = SessionTitles::disabled();
        titles.enabled = true;
        let mut fetches = 0;
        let first = titles.lookup("k", || {
            fetches += 1;
            Some("A".to_owned())
        });
        let second = titles.lookup("k", || {
            fetches += 1;
            Some("B".to_owned())
        });
        assert_eq!(first.as_deref(), Some("A"));
        assert_eq!(second.as_deref(), Some("A"));
        assert_eq!(fetches, 1);
    }

    #[test]
    fn disabled_resolver_never_looks_anything_up() {
        let mut titles = SessionTitles::disabled();
        assert_eq!(titles.claude("x", None), None);
        assert_eq!(titles.codex("x"), None);
    }
}
