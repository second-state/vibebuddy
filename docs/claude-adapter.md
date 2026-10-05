# Claude Code Live Adapter

## Scope

Claude Code's public lifecycle hooks provide everything Vibe Buddy needs, so the approach matches Codex: use only the official hooks, don't scrape the UI, don't parse the transcript.

| Claude Code Hook | Vibe Buddy state |
| --- | --- |
| `UserPromptSubmit` | Working; replaces the session's previous turn, but subagents still running keep their cards |
| `PermissionRequest` | Needs input |
| `PostToolUse` | Back to working |
| `SubagentStart` | Working; the subagent gets its own card |
| `SubagentStop` | That subagent's card goes away; not announced |
| `Stop` (the reply is waiting for the user to answer) | Needs input |
| `Stop` (any other reply) | Done; if the session still has subagents running, the turn ends quietly instead |
| `StopFailure` | Idle, titled `STOPPED`; not falsely reported as success |
| `SessionEnd` | Idle |

The rule for deciding whether the assistant is waiting for an answer is shared with Codex in [`hook/src/filter.rs`](../hook/src/filter.rs); see [ADR-0002](adr/0002-both-adapters-share-the-waiting-heuristic.md) for why. Permission waits don't go through that rule; `PermissionRequest` is an explicit event.

## Activity identity

Claude Code's `prompt_id` lines up semantically with Codex's `turn_id`: both identify a Turn.

Note that **background agents share the parent session's `session_id` and `prompt_id`**: a 2026-09-14 test showed that for the `SubagentStart` and `SubagentStop` fired by a subagent, and the tool events inside it, both fields are the same as the parent session's; only `agent_id` differs. So an activity's identity must be `session_id + prompt_id + agent_id`, or parallel subagents overwrite each other.

## Subagents are not the user's tasks

A subagent reports to its parent session, not to the user, so its end is never announced; its card just leaves the stack. The parent's work isn't done while any of its subagents still runs, either. With background agents, the parent often ends its turn to wait for them, and each completion notification (or other injected message) wakes it for another short turn. Each of those turns ends in a `Stop`, so before this rule one request with two background reviewers said "All done" five times (2026-10-05). Now a `Stop` with subagents still running ends the turn quietly, and only the turn that wraps up after the last subagent is announced.

The cost is that a lost `SubagentStop` would mute the parent until the card expires, so subagent cards expire after 10 minutes without an event instead of 30.

Background shell commands and scheduled wake-ups have no hooks, so the daemon can't see them: a turn that ends while one runs is still announced. Seeing them would mean parsing the transcript, which [ADR-0001](adr/0001-claude-adapter-does-not-parse-transcript.md) rules out.

## Coexisting with Codex

Both adapters write into the same aggregator, because the device has only one screen and one buddy. Task card titles carry an agent prefix: `CX:` for Codex, `CC:` for Claude Code. The two agents often work in the same directory, and without a prefix there's no way to tell which window to switch back to.

The prefix may only use characters the firmware font supports: `A-Z`, `0-9` and a few symbols such as `-.:/!?>`. The firmware renders single bytes, so a non-ASCII separator would be split into two unknown glyphs.

The title comes from the git project root rather than the working directory: using the working directory directly would show `repo/tools` as `TOOLS`, and a worktree as its branch directory name. A worktree's `.git` is a file pointing back to the main repository, so both cases resolve to the same project name.

## K2 navigation

Where K2 lands depends on **where the agent runs**: where the agent process actually lives.

| Where it runs | Test | K2 opens |
| --- | --- | --- |
| A Code session in the Claude App | `CLAUDE_CODE_HOST_SESSION_ID` is present | `claude://code/continue?session=<desktop session id>` |
| Another app (terminal, editor) | `__CFBundleIdentifier` is something else | `open -b <that bundle id>` |
| No host (SSH, background process) | neither is present | skip, try the next candidate |

The decision is made in the hook, the only place that can see the process environment; `vibebuddyd` only routes. The hook reports three derived fields, `surface`, `host_bundle_id` and `desktop_session_id`; they come from environment variables and contain no user content.

**The desktop session id is supplied by the Claude App itself.** Code sessions started by the App put it in `CLAUDE_CODE_HOST_SESSION_ID`, and the hook reports it as is:

```text
CLAUDE_CODE_HOST_SESSION_ID=local_44d42f48-a5cc-43c6-b95b-26407f579d39
```

The earlier approach took the CLI `session_id` and disambiguated by `cliSessionId` plus `cwd` in `~/Library/Application Support/Claude/claude-code-sessions/`. That mapping was one-to-many: worktree migration, forks and every `claude://resume` import each add another desktop record. It once opened a shadow session whose content stopped at the previous day, and every press re-imported the whole 5.4 MB transcript (see `LESSONS.md`). The environment variable is the authoritative identity given by the App; there's nothing to guess. The index scan is kept only as a fallback for old hooks and old state files.

**Terminal sessions don't use a deeplink.** `claude://resume` imports the terminal session into a copy inside the App, while the person is still in the terminal. Instead, the target is to bring the host app to the front, with the bundle id taken straight from `__CFBundleIdentifier`, which LaunchServices injects when it launches an app and which is inherited down the process chain to the hook.

Nothing here knows about any specific terminal: whatever bundle id is read gets opened. Ghostty, iTerm2, WezTerm, Terminal.app, and the integrated terminals in VS Code and Cursor all take the same path, and a terminal we've never seen needs no code change. `TERM_PROGRAM` is avoided precisely because it would require maintaining a table mapping terminal names to bundle ids.

**The Claude App's embedded terminal panel is an exception**: its `__CFBundleIdentifier` is also the Claude App, but it is a terminal scenario. It is told apart by the absence of `CLAUDE_CODE_HOST_SESSION_ID`: the CLI in the panel doesn't have it. In that case it is treated as a host, and K2 brings the Claude App to the front, landing the person on that panel.

**Don't look at the tty.** Agents run tool commands in non-interactive subprocesses, which report `not a tty` even when the host is a terminal (tested on 2026-09-21 running codex in Ghostty). The hook is also a subprocess and has no tty either.

On 2026-09-15 we verified with the local Claude 1.52386.6 that `code/continue` moves the target session's `lastFocusedAt` forward and produces no import logs.

## Privacy boundary

A hook's raw payload contains `prompt`, `tool_input`, `tool_response`, `transcript_path`, `session_title` and `last_assistant_message`. Before sending HTTP, the hook ([`hook/src/claude.rs`](../hook/src/claude.rs); originally `tools/claude-hook.py`) keeps only:

- `session_id`
- `prompt_id`
- `hook_event_name`
- `cwd`
- `agent_id`
- `agent_type`

There are also three derived fields that come from the process environment and contain no user content: `surface`, `host_bundle_id` and `desktop_session_id` (see the previous section).

If and only if the main session's `Stop` is judged by the local rule to be waiting for an answer, the derived field `response_kind` is added. A subagent's last paragraph is a report to its parent session, not a question to the user, so `SubagentStop` doesn't go through that check.

It only requests `http://127.0.0.1:7331/v1/claude-hooks`, with a 0.5-second timeout; if the daemon isn't running or the payload is invalid, it silently exits 0.

## Installation

In `~/.claude/settings.json` (applies to all projects) or a project's `.claude/settings.json`, configure the eight events in the table above:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/usr/bin/python3 /Users/dragon/workspace/vibe-buddy/tools/claude-hook.py",
            "timeout": 2
          }
        ]
      }
    ]
  }
}
```

Hooks stay synchronous; don't use `async`. Async would let events arrive out of order, and the activity model depends on order: a late `PostToolUse` would bring an already finished Turn back to life. The script itself has a 0.5-second timeout and fails silently; a normal round trip takes under 10 milliseconds. All `SessionEnd` hooks share a 1.5-second budget, and the 0.5-second cap is safely within it.

## vibebuddy-hook

Since 2026-09-16, hooks are handled by the Rust binary `vibebuddy-hook claude` (source in `hook/`). Its predecessor `tools/claude-hook.py` is retired, and its test cases were carried over one by one. The App copies it to `~/Library/Application Support/VibeBuddy/bin/vibebuddy-hook`, and the command in the configuration is written as `"<that path>" claude`. Use this path when configuring by hand too; don't point it inside the App bundle, or it breaks when the App moves (ADR-0005).
