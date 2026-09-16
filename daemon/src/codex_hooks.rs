//! Codex 生命周期事件到通用活动模型的映射。
//!
//! 这里只做翻译：任务卡聚合、去重、播报和过期都在 [`crate::activity`]。
//! Adapter 不持有状态，因为设备只有一块屏幕，所有 Agent 共享同一个聚合器。

use vibebuddy_protocol::Event;
use serde::Deserialize;

use crate::activity::{
    ActivityId, ActivitySource, ActivityStatus, ActivityTracker, card_title, display_title,
    project_name,
};
use crate::session_titles::{SessionTitles, git_branch};

/// 任务卡上区分 Agent 的前缀。
const PREFIX: &str = "CX:";
/// 工作目录不可用时的任务卡标题。
const FALLBACK_TITLE: &str = "CODEX";

#[derive(Debug, Deserialize)]
pub struct CodexHook {
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub response_kind: Option<String>,
}

pub fn apply(
    tracker: &mut ActivityTracker,
    titles: &mut SessionTitles,
    hook: CodexHook,
) -> Option<Event> {
    tracker.note_workspace(hook.cwd.as_deref());
    let id = activity_id(&hook);
    let cwd = hook.cwd.as_deref();
    // 第一行写 Codex 线程的名字（用户起的名或线程记录的分支），没有就写
    // 本地分支，再没有才是项目名。
    let project = project_name(cwd);
    let thread_id = hook.thread_id.clone().unwrap_or_else(|| hook.session_id.clone());
    // Codex 的后台会话（回合结束后生成 ambient suggestions 的那种）也触发
    // 同一套 Hook：没有工作目录，线程表里也没有它。它不是用户的活动，
    // 不该有卡片、不该播报，更不该成为 K2 的落点——打开它是一个空白会话。
    if project.is_none() && titles.codex_thread_known(&thread_id) == Some(false) {
        tracing::info!(
            session = %hook.session_id,
            event = %hook.hook_event_name,
            "忽略没有线程的 Codex 后台会话"
        );
        return None;
    }
    let candidates = [titles.codex(&thread_id), cwd.and_then(git_branch)];
    let title = card_title(PREFIX, &candidates, project.as_deref(), FALLBACK_TITLE);
    tracker.note_project(&id, &display_title("", project.as_deref().unwrap_or(FALLBACK_TITLE), FALLBACK_TITLE));
    let source = ActivitySource::Codex {
        thread_id: hook
            .thread_id
            .clone()
            .unwrap_or_else(|| hook.session_id.clone()),
    };
    let event = match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => {
            tracker.clear_session(&hook.session_id);
            tracker.observe(&id, &title, ActivityStatus::Working)
        }
        "PermissionRequest" => tracker.require_input(&id, &title),
        "PostToolUse" => tracker.observe(&id, &title, ActivityStatus::Working),
        "Stop" => {
            if hook.response_kind.as_deref() == Some("input_required") {
                tracker.require_input(&id, &title)
            } else {
                tracker.finish(&id, &title)
            }
        }
        "Interrupt" => tracker.discard(&id, "INTERRUPTED"),
        "SessionEnd" => tracker.discard_session(&hook.session_id, "ALL QUIET"),
        _ => None,
    };
    tracker.associate_source(&id, source);
    event
}

/// Codex 用 `session_id` 与 `turn_id` 合成活动身份。
fn activity_id(hook: &CodexHook) -> ActivityId {
    let key = match hook.turn_id.as_deref() {
        Some(turn_id) => format!("{}:{turn_id}", hook.session_id),
        None => hook.session_id.clone(),
    };
    ActivityId {
        session_id: hook.session_id.clone(),
        key,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn background_sessions_without_a_thread_are_ignored() {
        let dir = std::env::temp_dir().join(format!("codex-ghost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.sqlite");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, git_branch TEXT);
                 INSERT INTO threads VALUES ('real-thread', 'Review', NULL);",
            )
            .unwrap();
        let mut titles = SessionTitles::with_codex_db(db);
        let mut tracker = ActivityTracker::default();

        let ghost = CodexHook {
            session_id: "ghost".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("ghost".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: None,
            response_kind: None,
        };
        assert!(apply(&mut tracker, &mut titles, ghost).is_none());
        assert!(tracker.focus_source().is_none());

        // 有工作目录的会话照常，即便线程表暂时还没有它。
        let fresh = CodexHook {
            session_id: "fresh".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("fresh".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            response_kind: None,
        };
        assert!(apply(&mut tracker, &mut titles, fresh).is_some());

        // 线程表里有的会话，没有工作目录也算数。
        let known = CodexHook {
            session_id: "real-thread".to_owned(),
            turn_id: Some("t1".to_owned()),
            thread_id: Some("real-thread".to_owned()),
            hook_event_name: "UserPromptSubmit".to_owned(),
            cwd: None,
            response_kind: None,
        };
        let event = apply(&mut tracker, &mut titles, known).expect("已知线程应可见");
        assert_eq!(event.title.as_deref(), Some("CX:REVIEW"));
    }

    fn hook(session: &str, name: &str, cwd: &str) -> CodexHook {
        hook_with_turn(session, &format!("{session}-turn"), name, cwd)
    }

    fn hook_with_turn(session: &str, turn: &str, name: &str, cwd: &str) -> CodexHook {
        CodexHook {
            session_id: session.to_owned(),
            turn_id: Some(turn.to_owned()),
            thread_id: None,
            hook_event_name: name.to_owned(),
            cwd: Some(cwd.to_owned()),
            response_kind: None,
        }
    }

    #[test]
    fn codex_source_uses_explicit_navigable_thread_id() {
        let mut tracker = ActivityTracker::default();
        let hook: CodexHook = serde_json::from_value(json!({
            "session_id": "child-session",
            "turn_id": "child-turn",
            "thread_id": "parent-thread",
            "hook_event_name": "UserPromptSubmit",
            "cwd": "/work/memories"
        }))
        .expect("带可导航线程的 Hook 应可解析");

        apply(&mut tracker, &mut SessionTitles::disabled(), hook);

        assert_eq!(
            tracker.focus_source(),
            Some(ActivitySource::Codex {
                thread_id: "parent-thread".to_owned(),
            })
        );
    }

    #[test]
    fn maps_codex_lifecycle_without_prompt_content() {
        let mut tracker = ActivityTracker::default();

        let working = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/vibe-buddy"),
        )
        .expect("开始事件应可见");
        assert_eq!(working.event, "task.start");
        assert_eq!(working.title.as_deref(), Some("CX:VIBE-BUDDY"));

        let waiting = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "PermissionRequest", "/work/vibe-buddy"),
        )
        .expect("审批事件应可见");
        assert_eq!(waiting.event, "agent.input_required");

        let resumed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "PostToolUse", "/work/vibe-buddy"),
        )
        .expect("工具完成后应恢复工作中");
        assert_eq!(resumed.event, "task.start");

        let done = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "Stop", "/work/vibe-buddy"),
        )
        .expect("停止事件应可见");
        assert_eq!(done.event, "task.done");
    }

    #[test]
    fn stop_that_waits_for_a_reply_requests_input_instead_of_reporting_done() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/vibe-buddy"),
        );
        let stop: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "session-a-turn",
            "hook_event_name": "Stop",
            "cwd": "/work/vibe-buddy",
            "response_kind": "input_required"
        }))
        .expect("等待回答的 Stop 载荷应可解析");

        let waiting = apply(&mut tracker, &mut SessionTitles::disabled(), stop).expect("等待回答应产生可见事件");
        assert_eq!(waiting.event, "agent.input_required");
        assert!(!waiting.extra.contains_key("announcement"));
    }

    #[test]
    fn input_required_has_priority_over_other_work() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("working", "UserPromptSubmit", "/work/alpha"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/beta"),
        );

        let waiting = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/beta"),
        )
        .expect("需要输入应成为可见状态");
        assert_eq!(waiting.event, "agent.input_required");
        assert_eq!(waiting.title.as_deref(), Some("CX:BETA"));

        // 结束播报说的是谁结束了，屏幕就显示谁；alpha 还在跑，由它后续的事件
        // 把屏幕刷回去。
        let done =
            apply(&mut tracker, &mut SessionTitles::disabled(), hook("waiting", "Stop", "/work/beta")).expect("结束应产生播报");
        assert_eq!(done.event, "task.done");
        assert_eq!(done.title.as_deref(), Some("CX:BETA"));

        let back = apply(&mut tracker, &mut SessionTitles::disabled(), hook("working", "PostToolUse", "/work/alpha"))
            .expect("下一个事件应把屏幕交还给还在跑的任务");
        assert_eq!(back.event, "task.start");
        assert_eq!(back.title.as_deref(), Some("CX:ALPHA"));
    }

    /// 一个任务完成时，另一个正等着人回答：屏幕必须留给等回答的那个，它要用
    /// 户动手；"完成"播报一声就够了。
    #[test]
    fn finishing_one_task_does_not_hide_another_waiting_for_a_reply() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("finishing", "UserPromptSubmit", "/work/alpha"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/beta"),
        );
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/beta"),
        );

        let done =
            apply(&mut tracker, &mut SessionTitles::disabled(), hook("finishing", "Stop", "/work/alpha")).expect("结束应产生播报");

        assert_eq!(
            done.extra.get("announcement").and_then(|v| v.as_str()),
            Some("done"),
            "完成仍然要播报"
        );
        assert_eq!(done.event, "agent.input_required");
        assert_eq!(
            done.title.as_deref(),
            Some("CX:BETA"),
            "屏幕该留给等人回答的那个"
        );
    }

    #[test]
    fn background_refresh_does_not_reannounce_existing_input_request() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "UserPromptSubmit", "/work/waiting"),
        );
        let first = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("waiting", "PermissionRequest", "/work/waiting"),
        )
        .expect("首次等待输入应可见");
        assert!(!first.extra.contains_key("suppress_audio"));

        let refreshed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("working", "UserPromptSubmit", "/work/working"),
        )
        .expect("后台任务变化应刷新卡片");
        assert_eq!(refreshed.event, "agent.input_required");
        assert_eq!(refreshed.extra["suppress_audio"], true);
    }

    #[test]
    fn task_cards_are_newest_first_and_limited_to_three() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("one", "UserPromptSubmit", "/work/one"));
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("two", "UserPromptSubmit", "/work/two"));
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("three", "UserPromptSubmit", "/work/three"),
        );
        let latest = apply(&mut tracker, &mut SessionTitles::disabled(), hook("four", "UserPromptSubmit", "/work/four"))
            .expect("新任务应刷新卡片栈");

        let tasks = latest.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0]["title"], "CX:FOUR");
        assert_eq!(tasks[1]["title"], "CX:THREE");
        assert_eq!(tasks[2]["title"], "CX:TWO");
    }

    #[test]
    fn background_task_removal_refreshes_the_stack() {
        let mut tracker = ActivityTracker::default();
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("old", "UserPromptSubmit", "/work/old"));
        apply(&mut tracker, &mut SessionTitles::disabled(), hook("new", "UserPromptSubmit", "/work/new"));

        let refreshed = apply(&mut tracker, &mut SessionTitles::disabled(), hook("old", "Stop", "/work/old"))
            .expect("后台任务结束也应刷新卡片栈");
        let tasks = refreshed.extra["tasks"].as_array().expect("tasks 应为数组");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "CX:NEW");
    }

    #[test]
    fn duplicate_visible_state_is_suppressed() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/alpha"),
        );

        assert!(
            apply(
                &mut tracker,
                &mut SessionTitles::disabled(),
                hook("session-a", "PostToolUse", "/work/alpha")
            )
            .is_none()
        );
    }

    #[test]
    fn reply_starts_a_new_turn_and_clears_the_previous_waiting_turn() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-a", "UserPromptSubmit", "/work/alpha"),
        );
        let waiting: CodexHook = serde_json::from_value(json!({
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/alpha",
            "response_kind": "input_required"
        }))
        .expect("等待回答的 Stop 载荷应可解析");
        assert_eq!(
            apply(&mut tracker, &mut SessionTitles::disabled(), waiting).expect("提问应等待回答").event,
            "agent.input_required"
        );

        let resumed = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-b", "UserPromptSubmit", "/work/beta"),
        )
        .expect("用户回答后应开始新 turn");
        assert_eq!(resumed.event, "task.start");
        assert_eq!(resumed.title.as_deref(), Some("CX:BETA"));

        let replay = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-a", "Stop", "/work/alpha"),
        );
        assert!(replay.is_none(), "旧 turn 的重复 Stop 不应改变新 turn");

        let second = apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook_with_turn("session-a", "turn-b", "Stop", "/work/beta"),
        )
        .expect("第二个 turn 应产生完成通知");
        assert_eq!(second.event, "task.done");
    }

    #[test]
    fn interrupt_does_not_report_success() {
        let mut tracker = ActivityTracker::default();
        apply(
            &mut tracker,
            &mut SessionTitles::disabled(),
            hook("session-a", "UserPromptSubmit", "/work/alpha"),
        );

        let interrupted = apply(&mut tracker, &mut SessionTitles::disabled(), hook("session-a", "Interrupt", "/work/alpha"))
            .expect("中断应回到空闲状态");
        assert_eq!(interrupted.event, "agent.idle");
        assert_eq!(interrupted.title.as_deref(), Some("INTERRUPTED"));
    }

    #[test]
    fn every_tracked_stop_announces_completion_with_parallel_tasks() {
        let mut tracker = ActivityTracker::default();
        for session in ["one", "two", "three"] {
            apply(
                &mut tracker,
                &mut SessionTitles::disabled(),
                hook(session, "UserPromptSubmit", "/work/project"),
            );
        }

        let announcements = ["one", "two", "three"]
            .into_iter()
            .filter(|session| {
                let event = apply(&mut tracker, &mut SessionTitles::disabled(), hook(session, "Stop", "/work/project"))
                    .expect("每个活动会话结束都应产生事件");
                event.event == "task.done"
                    || event
                        .extra
                        .get("announcement")
                        .and_then(|value| value.as_str())
                        == Some("done")
            })
            .count();

        assert_eq!(announcements, 3, "三个会话应分别触发三次完成播报");
    }
}
