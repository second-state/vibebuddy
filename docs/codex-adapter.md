# Codex Live Adapter

## Scope

Codex's built-in pet assets, animation state machine and task card rendering are not exposed as a pet API, so they can't be reliably mirrored frame by frame to an external device. The public interface we can depend on is Codex's lifecycle hooks.

VibeBuddy uses this mapping:

| Codex Hook | VibeBuddy state |
| --- | --- |
| `UserPromptSubmit` | Working |
| `PermissionRequest` | Needs input |
| `PostToolUse` | Back to working |
| `Stop` (the reply is waiting for the user to answer) | Needs input |
| `Stop` (any other reply) | Done |
| `Interrupt` | Idle, titled `INTERRUPTED`; not falsely reported as success |
| `SessionEnd` | Idle |

`vibebuddyd` aggregates multiple sessions: task cards are ordered by most recent activity, up to 3; a session that needs input takes priority in controlling the pet's expression. An activity's identity is determined by `session_id` together with `turn_id`. For every tracked turn that isn't waiting for the user to answer, its `Stop` produces one completion notification, even if the screen still needs to show other tasks that are working; repeated `Stop`s for the same turn are not announced again. A turn that is waiting for an answer stays in needs input and switches back to working when the user submits the next message. When other tasks cause the cards to refresh, the screen still shows needs input, but the same input reminder is not played again.

When Codex is force-quit it doesn't send `SessionEnd`, so activities must also be able to expire on their own. `vibebuddyd` scans every 60 seconds: a working activity with no hook events for over 30 minutes is treated as a vanished process, while a needs-input activity is kept for 4 hours so that reminders aren't lost while the user is away. When the last activity is cleared, it sends an `agent.idle` titled `TIMED OUT`, with no voice.

Hardware result (2026-09-14): with the timeouts temporarily shortened to seconds, an abandoned working activity was cleared by the background scan after it timed out, and the device reported `TITLE TIMED OUT` and `DISPLAY STATE READY`, with no `AUDIO QUEUED`; an activity waiting for input survived five scans past the working timeout and was released only after exceeding its own timeout. The firmware was not modified or reflashed.

Codex hooks don't provide a structured field saying "this assistant reply asks the user to answer". `Stop` provides `last_assistant_message`; the privacy-filter script checks, locally only, whether the last paragraph contains an explicit question or a request for a reply, and then emits `response_kind: input_required`. This is a conservative text rule, not a remote semantic analysis of the reply body.

The English waiting heuristic shared by both adapters ignores Markdown emphasis markers in optional sign-offs and trailing punctuation or emoji without textual content: `**Let me know** if you need anything else.`, `Just let me know…` and `Just let me know :)` do not require input. Formatted concrete requests still require input.

## K2 navigation and where the agent runs

Where K2 lands depends on where the agent runs; the rules are shared with Claude Code (see [`claude-adapter.md`](claude-adapter.md#k2-navigation)):

| Where it runs | Test | K2 opens |
| --- | --- | --- |
| Codex App | `__CFBundleIdentifier` is `com.openai.codex` | `codex://threads/<thread_id>` |
| Another app (terminal, editor) | `__CFBundleIdentifier` is something else | `open -b <that bundle id>` |
| No host (SSH, background process) | no `__CFBundleIdentifier` | skip, try the next candidate |

The Codex App is installed inside `ChatGPT.app`, and the `codex:` scheme is claimed by that bundle too (checked with `lsregister` on 2026-09-21).

Codex injects its own `CODEX_SESSION_ID` and `CODEX_THREAD_ID`, but they are no use for telling where it runs: sessions in the App most likely have them too. The decision looks only at `__CFBundleIdentifier`.

Codex has no second identity like `CLAUDE_CODE_HOST_SESSION_ID`, so there is no way to correct the case of "running in the Codex App's own terminal panel": it is judged to be an App session. The consequence is limited: K2 still brings the Codex App to the front, it just lands on the thread view rather than that panel.

## Privacy boundary

A hook's raw JSON may contain the prompt, the transcript path, tool input and tool output. Before sending HTTP, `tools/codex-hook.py` keeps only:

- `session_id`
- `turn_id`
- `thread_id` (the K2 navigation target; for a top-level session it equals `session_id`, and a subagent maps to the parent session that owns it)
- `hook_event_name`
- `cwd`

If and only if a `Stop` is judged by the local rule to be waiting for an answer, the script also adds the derived field `response_kind`. `last_assistant_message` itself is never sent to the daemon.

There are also two derived fields that come from the process environment and contain no user content: `surface` and `host_bundle_id` (see the previous section).

It only requests `http://127.0.0.1:7331/v1/codex-hooks`, with a 0.5-second timeout; if the daemon isn't running, the payload is invalid or the connection fails, it silently exits 0 and doesn't block Codex. Don't change it to a plain `curl --data-binary @-`, or sensitive fields will cross the adapter boundary.

## Installation and trust

The user-level `~/.codex/hooks.json` calls this for the six events above:

```text
/usr/bin/python3 /Users/dragon/workspace/vibe-buddy/tools/codex-hook.py
```

When the hook configuration is added or changed, Codex requires a new review keyed to the exact hash of the definition. Open `/hooks`, check the script path and the six events, then trust it; don't bypass the trust mechanism. Only new sessions or a reloaded Codex use the new configuration.

A Codex subagent has its own lifecycle `session_id`, but that internal thread can't necessarily be opened by a desktop deeplink. The hook script reads only the `session_meta` on the first line of the transcript and derives `thread_id` locally from `source.subagent.thread_spawn.parent_thread_id`; neither the content nor the transcript path is sent to `vibebuddyd`. Activity deduplication still uses the subagent's `session_id + turn_id`; only K2 navigation uses the parent thread. Don't mix these two identities.

## Running in the background

## Background sessions

Codex fires the same set of hooks for its own background sessions, typically the run that generates ambient suggestions after each turn ends: it has no working directory, and its id isn't in the `threads` table of `~/.codex/state_5.sqlite` either. In a 2026-09-16 test it finished right after a real turn, so the device played "task complete" twice in a row, and K2 treated it as the most recently announced item and opened `codex://threads/<id>`, which gave a blank session.

So `vibebuddyd` filters at the entry point: any Codex session that has no usable working directory and can't be found in the threads table is ignored (with one info log line): no card, no announcement, no K2 target. Both conditions are required: a freshly created real session may not have been written to the threads table yet, but it has a working directory. Before K2 opens a thread it checks once more that the thread exists, and skips to the next candidate if not. `tools/codex-hook.py` also writes one diagnostic line without the prompt to `~/Library/Logs/VibeBuddy/codex-hooks.log` (event name, session and thread ids, directory, whether there is a transcript), so the next time a session of unknown origin shows up you can see what the hook actually received.

The `vibebuddyd` release binary is started automatically after login by `~/Library/LaunchAgents/com.vibebuddy.vibebuddyd.plist` and restarted if it exits abnormally. Logs go to `~/Library/Logs/VibeBuddy/vibebuddyd.log`. When the USB device is temporarily absent, the process keeps running and periodically rediscovers it; hooks don't need to know about plugging and unplugging.

After updating the daemon, run `cargo build --release -p vibebuddyd`, then restart the service with `launchctl kickstart -k gui/502/com.vibebuddy.vibebuddyd`. Stop the service before flashing firmware so it doesn't hold the serial port.

## Why not scrape the UI or the transcript

- UI pixels and the accessibility tree are not a lifecycle protocol, and they change between versions.
- Codex officially states that the transcript format is not a stable interface for hooks.
- Hooks give semantic events directly, with less data and lower latency, and they are easier to test.

If the Codex App Server someday offers a stable desktop session stream that can be attached to, a second adapter could be added to distinguish `systemError`, waiting for approval and waiting for input; the current desktop process uses a stdio subprocess, so we can't assume an external daemon can attach.

## vibebuddy-hook

Since 2026-09-16, hooks are handled by the Rust binary `vibebuddy-hook codex` (source in `hook/`). Its predecessor `tools/codex-hook.py` is retired, and its test cases were carried over one by one. The App copies it to `~/Library/Application Support/VibeBuddy/bin/vibebuddy-hook`, and the command in the configuration is written as `"<that path>" codex`. Use this path when configuring by hand too; don't point it inside the App bundle, or it breaks when the App moves (ADR-0005).
