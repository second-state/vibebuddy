# Plan: Add Kimi Code CLI Support to VibeBuddy

## Summary

Add Kimi Code CLI as a supported Agent through VibeBuddy’s existing generic CLI Adapter path. Connect, Repair, and Remove will manage VibeBuddy’s hooks in Kimi’s shared TOML config, and the Mac and Linux Apps will expose the same controls and status.

Kimi’s hooks are documented as beta. Validate the behavior against the supported Kimi CLI version before release. The initial integration treats `Stop` as done; it does not inspect session files or forward Kimi’s reply content.

## Implementation Changes

- **Hook Adapter and config management:** Add Kimi to the shared Rust hook binary’s supported-agent registry. Parse hook JSON from stdin, allowlist only session ID, event name, working directory, and required local host metadata, and normalize Kimi lifecycle events into VibeBuddy’s generic agent events. Add safe, previewable management of Kimi’s `config.toml`: preserve unrelated settings and hooks, back up before changes, write atomically, and remove only VibeBuddy’s entries. Resolve and display the effective config path, including `KIMI_CODE_HOME`. Keep the hook observation-only, bounded, and fail-open so delivery problems cannot block Kimi.

- **Lifecycle and activity behavior:** Map `UserPromptSubmit` and `PostToolUse` to working, `PermissionRequest` to needs input, `Stop` to done, `StopFailure` and `Interrupt` to stopped, and `SessionEnd` to session end. Ignore session-start, compaction, notification, and subagent events initially. Use the project name for the task card; do not add Kimi session titles to the daemon payload. Reuse existing generic activity identity, expiry, and Linux host-focus behavior. Do not promise exact-session K2 navigation.

- **Daemon and both Apps:** Add `kimi` to the daemon’s generic-agent allowlist and task-card identity, and track Kimi’s last hook event in status. Add Kimi to the Mac and Linux Agents lists, status extraction, icon/prefix display, and Connect/Repair/Remove preview flows. Keep English UI keys and add the corresponding Simplified Chinese translations on both platforms. Update the Linux installer and user-facing supported-agent documentation; keep README language versions aligned.

## Tests and Acceptance

- Verify TOML planning, installation, repair, and removal preserve unrelated tables, comments where supported, and other hook entries; cover absent, malformed, custom-home, and concurrently modified configs.
- Verify spaces and shell metacharacters in hook-binary paths cannot alter the generated command. Confirm hook failures and daemon unavailability do not block Kimi or return meaningful hook output.
- Feed representative Kimi payloads containing prompts, tool input/output, errors, and subagent content through the hook. Confirm only allowlisted fields reach VibeBuddy.
- Verify every selected event maps correctly, unknown events are ignored, repeated events do not duplicate completion announcements, and SessionEnd clears activities.
- Verify Kimi appears consistently in status and management flows on Mac and Linux, including localized strings and the resolved config path in the change preview.
- Smoke-test the supported Kimi CLI release on Linux and Mac: connect, generate work and permission events, finish and interrupt turns, remove hooks, and confirm Kimi’s config and normal operation remain intact.

## Assumptions and Defaults

- Treat Kimi `Stop` as done. Do not inspect Kimi session files to infer whether a conversational reply asks a question.
- Show the project name on task cards; do not expose `session_title` to the daemon.
- Manage hooks in Kimi’s shared TOML config through previewed, selective edits; manual-only setup is out of scope.
- Initially ignore Kimi subagent events and do not add exact-session deep-link behavior.
- Support Kimi on Mac and Linux together to maintain VibeBuddy’s cross-platform product parity.
