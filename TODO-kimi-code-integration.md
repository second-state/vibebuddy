# Kimi Code CLI Integration: Remaining Work

- [ ] Verify the complete Linux flow with a real Kimi session, including approval, interruption, K2 focus, and removal.
  - [x] Exercise all seven normalized events through a real loopback HTTP receiver (`hook/tests/kimi_smoke.py`).
  - [x] Verify consecutive turns, completion deduplication and session cleanup in the daemon tracker and over the daemon's real HTTP router.
  - [x] Run Kimi 2.4.1 print and ACP scenarios against a scripted local model with exact event sequences, a verified `pwd` tool result, approval, and cancellation (`hook/tests/kimi_cli_smoke.py`).
  - [x] Run the same kind of turns through a real `vibebuddyd` and read its status: done counts, `needs_input` and `working` shown while they hold (`hook/tests/kimi_daemon_smoke.py`).
  - [x] Run a real session on 2026-11-05: Kimi 2.5.0 TUI, real model, real daemon, XGGF on USB, real config. Plain, approved, rejected and interrupted turns; `/exit`; daemon stopped; repair; removal back to the identical file.
  - [ ] Look at the box's screen during a turn. The run only has the device's `STATE` replies; this firmware has no screenshot.
  - [ ] Verify graphical K2 focus on Linux. GNOME on Wayland can at most switch a tmux pane, and tmux is not installed on this host.
- [ ] Build and test the Mac App, including preview and config-change error handling.
  - [ ] Requires a macOS host with Swift/Xcode; neither tool is available on this Linux host.
  - [x] Add explicit preview error display and config snapshot checking before applying.
- [ ] Installer safety coverage for missing and custom config paths, concurrent edits, shell quoting, daemon failures, permissions, and old binary paths.
  - [x] Reject a changed config between planning and writing, both before the new text is staged and after it (the staged-window test injects the change).
  - [x] Preserve every other setting unchanged, not just comments and other hook entries; the test compares each top-level setting.
  - [x] Keep the backup private (0600 or the original mode) and equal to the text that was checked.
  - [x] Repair hook commands that point to an old VibeBuddy binary path.
  - [x] Create a missing config with private permissions and without a backup; a file created after planning is not overwritten.
  - [x] Verify shell quoting with spaces, quotes, dollar signs, and semicolons.
  - [x] Exercise a custom Kimi home through plan, install, Kimi 2.4.1 `doctor`, and uninstall.
  - [x] Exercise daemon failures end to end: the hook exits successfully and silently within two seconds with no listener, and within the same bound when the daemon accepts and never answers.
  - [ ] Residual race: a change in the few microseconds between the last check and the rename is not refused. POSIX has no compare-and-swap for files. Accept this and document it, or add locking that Kimi also honours.
- [ ] Finish both Apps' icon mappings and user-facing documentation; verify localization and UI behavior.
  - [x] Use the same explicit generic Agent icon on both platforms and task-card previews.
  - [x] Update both READMEs, architecture and Linux installer description; add docs/kimi-adapter.md.
  - [x] Pass localization check (608 keys).
  - [x] Pass the hook crate's 62 tests and the full daemon suite (436 passed, 3 ignored as before).
  - [ ] Verify graphical UI interaction on Linux and macOS.
- [x] Validate Kimi event ordering, approval resolution, and supported CLI versions with real sessions.
  - [x] Validate generated config using Kimi Code CLI 2.4.1 doctor; document version scope.
  - [x] Review PermissionResult and PostToolUseFailure in the local Kimi source and document current limitations.
  - [x] Validate asynchronous delivery through installed Kimi CLI/ACP 2.4.1 with a scripted local model, with exact sequences: working → needs_input → working → done for approval; working → stopped for cancellation and model errors.
- [ ] Run the Kimi-backed checks in CI.
  - [x] The Kimi-free hook smoke runs in CI (`ci.yml`, "Kimi hook smoke"); the daemon's HTTP path runs in the Rust test suite.
  - [ ] The Kimi CLI and daemon smokes are not in CI: the Kimi CLI comes from a self-updating CDN and the repository pins no installer. Add a pinned install, or a repository-hosted binary, before registering them.
- [x] Confirm the `SessionEnd` path with the real Kimi CLI.
  - [x] Kimi 2.4.1 sent no `SessionEnd` in print, ACP-prompt, failure, or ACP-cancellation runs, and the exact-sequence checks would have failed if it had.
  - [x] `/exit` in the interactive TUI on 2.5.0 posted a Kimi event from an idle session, twice. `SessionEnd` is the only subscribed event that fits; the daemon does not record which one arrived. A graceful ACP shutdown is still untested.
- [ ] Fix the Repair preview: with only `Stop` missing, `plan install kimi` still lists all seven hooks as `+`. The written config is correct (seven hooks, none duplicated).
- [ ] Decide whether a turn whose tool call the user rejected should be announced as done. It is today, and `docs/kimi-adapter.md` says so.
- [x] Remove a throwaway computer key from the Cardputer. A daemon started without the serial pin paired the box with a temporary identity, key `wFUp-42hbtGMokEX4SNoz0HVZvMYOOU1mIET4fPRyrM`. Removed on 2026-11-05 through `/v1/device/unpair`; the box's list no longer has it. The daemon smokes now pin the box search, so this does not happen again.

## Latest checks

- Hook crate: 62 tests pass (was 57). New: installer preservation of every other setting, refusal of a change inside the staged window, refusal of a file created inside it, private backup; payload allowlist with a marker in every sensitive field.
- Daemon: full suite 436 passed, 3 ignored (as before). New Kimi test: the hook's payloads through the real router over HTTP, with `needs_input` and `working` checked after each step, one announcement per turn, and cleanup at session end.
- `kimi_smoke.py` passes (CI step): one POST per mapped event with exact fields, sensitive content withheld, ignored and malformed input posts nothing, missing and silent daemon return within two seconds.
- `kimi_cli_smoke.py` passes against Kimi 2.4.1: exact sequences for text, tool, ask-rule, failure, ACP approval and ACP cancellation; verified `pwd` results; reported session IDs match ACP and Kimi's own session directories; installing keeps existing settings byte for byte.
- `kimi_daemon_smoke.py` passes against a real `vibebuddyd` with the serial port pinned: device not connected, done counts exact, `needs_input` and `working` seen on `/v1/status`, failed and cancelled turns not announced.
- Mutation checks: eight deliberate faults, each applied to the working tree and restored byte for byte. Every fault now fails for its intended reason in at least one layer: dropped `PostToolUse` (sequence), corrupted session ID (session cross-check), deleted unrelated setting (installer test and config-prefix check), removed check before the swap (installer staged-window test), forwarded prompt (allowlist), daemon ignoring session end (daemon unit and HTTP tests), hook posting twice (hook smoke and sequence; the daemon smoke still passes because the tracker absorbs the duplicate, which is correct), daemon dropping `needs_input` (HTTP state check and daemon smoke).
- Review gaps fixed: the smokes counted other agents' posts on port 7331 and could fail or pass for the wrong reason; they now count only Kimi's posts, and the daemon smoke refuses to run when another agent reports during it.
- Localization passed with 608 keys.

## Remaining acceptance requirements

The TODO stays open until graphical K2/UI checks and macOS acceptance pass.
The Kimi-backed checks run locally against Kimi 2.4.1; they are not yet in CI.
macOS acceptance needs a Mac with the Swift toolchain; this Linux host has
neither Swift nor Xcode.

No live user Kimi configuration was changed. These automated checks do not stand in for graphical platform acceptance.
