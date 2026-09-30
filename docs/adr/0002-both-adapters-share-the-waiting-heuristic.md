---
status: accepted
---

# Both adapters share the same waiting heuristic

A test on 2026-09-14 showed that Claude Code's `Stop` event provides `last_assistant_message`, just like Codex; `SubagentStop` provides it as well. So the Claude adapter and the Codex adapter use the same local text rule to decide whether the assistant is waiting for an answer, and on top of that the Claude adapter also uses the explicit `PermissionRequest` signal.

## Why this supersedes ADR-0001

ADR-0001 assumed that Claude Code's `Stop` provides only `transcript_path`. That assumption was inferred from gaps in the official documentation, and testing proved it false. Since the payload already has `last_assistant_message`, there's no need to read the transcript, nor to accept the silent failure of "a question announced as done".

## Consequences

The two adapters behave the same visibly, and there is no reliability asymmetry. The Claude side actually has more information: permission requests have an explicit event, and conversational questions get the text check.

The cost is that the text rule from issue #1 goes from a local problem of one adapter to a shared dependency of both, so its misjudgments affect both pipelines at once, and it matters correspondingly more.

## Lesson

Facts about external interfaces must be tested, not taken from secondhand inferences about the documentation. Documentation not mentioning a field doesn't mean the field doesn't exist.
