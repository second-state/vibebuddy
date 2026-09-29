---
status: superseded by ADR-0002
---

# The Claude adapter doesn't parse the transcript, and accepts the semantic loss on `Stop`

Codex's `Stop` provides `last_assistant_message`, so the Codex adapter can tell locally whether the assistant is asking a question or has finished; Claude Code's `Stop` provides only `transcript_path`, so getting the same information would mean reading the session file. We decided not to read it: the Claude adapter maps every `Stop` to done, and "needs input" relies entirely on the explicit `PermissionRequest` signal.

## Relation to key decision 7

Key decision 7 in the architecture forbids parsing the unstable transcript, on the grounds that Codex officially states the transcript format is not a stable interface for hooks. Claude Code's `transcript_path` is a field the hook payload provides on purpose, with no such statement, so decision 7 doesn't directly forbid this path. This decision is not a consequence of decision 7's constraint but an independent trade-off.

## Rejected option

Read the last assistant message from `transcript_path` and reuse the text rule in `codex-hook.py`. This would make the two adapters behave the same, and parse failures could degrade gracefully. It was rejected because it would double the guessing surface of issue #1 and take on an extra file-format risk, while at this point no data showed that such guessing was necessary.

## Consequences

`PermissionRequest` covers the vast majority of situations in Claude Code that need the user to step in, but a purely conversational question gets announced as "task complete". **This is a silent failure**: the user thinks the work is over, and the device won't remind them again. Someone who sees this is likely to treat it as a bug and "fix" it; it is a deliberately accepted cost.

As a result, the two adapters are not equally reliable: on the Claude side "needs input" comes from an explicit signal and is more reliable than the text guessing on the Codex side, while "done" on the Claude side is more prone to false reports than on the Codex side.

The trigger for re-evaluating is false reports actually causing trouble. At that point it should be evaluated together with `Notification`'s `idle_prompt`, and the text rule should be tested against real samples.
