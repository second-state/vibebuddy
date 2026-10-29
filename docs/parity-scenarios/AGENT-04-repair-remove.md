# AGENT-04: Repair or remove an agent's connection

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`HookRow`)
- `app/Sources/VibeBuddy/HookInstaller.swift` (`installPlan`, `removePlan`, `apply`)

## Initial state (input)

- The agent is connected.

## Trigger

"Repair" or "Remove".

## Workflow (Mac app)

1. Repair shows the same plan as Connect; when nothing differs it says "Nothing to change." and Write is disabled.
2. Remove, for Claude Code and Codex: a plan that drops only VibeBuddy's entries (old names count as ours) and any events or `hooks` objects left empty; a `.bak` is kept.
3. Remove, for OpenCode, Copilot and Pi: the plan shows the file's lines as removed; Write deletes the file.
4. The rows refresh after writing.

## Final state (output)

- Repaired: all events point at the current binary. Removed: no VibeBuddy hook remains; the user's own hooks are intact.

## Check on Linux

- [ ] Same Repair and Remove semantics, with the change shown first?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
