//! 任务卡第一行的来源：Agent 自己给会话起的名字。
//!
//! 项目名分不开同一个项目里的几件事。两个 Agent 都把会话名存在本机：
//! Claude App 的会话索引有它自动生成的 `title`，Codex 的 `state_5.sqlite`
//! 有用户起的 `name` 和线程记录的 `git_branch`。这些都不是 prompt：前者是
//! App 侧栏里本来就显示的摘要，后者是用户自己敲的名字。Codex 的 `title`
//! 列是原始首条消息（还混着注入的上下文），不用。
//!
//! 查询有缓存：Hook 每几秒来一次，不能每次都扫目录、开数据库。标题可能在
//! 会话开始后一会儿才生成，所以没查到的每 30 秒再试，查到的每 5 分钟刷新。

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
    /// 线程是否存在于 Codex 的线程表；没查到的也缓存，免得后台会话每个
    /// Hook 都开一次数据库。
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

    /// 测试用：什么都查不到，标题退回分支或项目名，线程一律当作存在。
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            codex_db: None,
            cache: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// 测试用：指定 Codex 状态库的位置。
    #[cfg(test)]
    pub fn with_codex_db(db: PathBuf) -> Self {
        Self {
            enabled: true,
            codex_db: Some(db),
            cache: HashMap::new(),
            known: HashMap::new(),
        }
    }

    /// 这个线程是否存在于 Codex 的线程表。查不了（库不在、打不开）返回 None，
    /// 调用方按“存在”处理：宁可多显示一个后台会话，也不能把真会话滤掉。
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

    /// Claude App 给这个会话起的标题。终端里直接跑的会话没有记录。
    pub fn claude(&mut self, cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let key = format!("claude:{cli_session_id}");
        self.lookup(&key, || {
            source_opener::desktop_session_title(cli_session_id, cwd)
        })
    }

    /// Codex 线程的名字：用户起的 `name` 优先，其次线程记录的分支。
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

/// 只读打开 Codex 的状态库。Codex 自己在写它（WAL），只读连接不会碍事；
/// 任何失败都当作没有标题。
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

/// 线程表里有没有这个 id。库打不开时返回 None。
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

/// 工作目录所在的分支；主分支说明不了任务，不算。
///
/// worktree 的 `.git` 是文件，指向主仓库里这个 worktree 自己的目录，那里的
/// `HEAD` 才是它的分支——不能用 `project_root` 回到主仓库去读。
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

/// `claude/pomodoro-timer-feature` 只留最后一段：前缀说的是谁开的分支，
/// 卡片上放不下也不需要。
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
