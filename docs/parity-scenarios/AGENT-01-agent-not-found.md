# AGENT-01: An agent isn't installed on this computer

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`HookRow`)
- `app/Sources/VibeBuddy/HookInstaller.swift` (`isPresent`)

## Initial state (input)

- The agent's own folder doesn't exist (`~/.codex`, `~/.claude`, `~/.config/opencode`, `~/.copilot` or `~/.pi/agent`).

## Trigger

The Agents tab is shown.

## Workflow (Mac app)

1. The row has a grey dot and says "{Agent} wasn't found on this Mac".
2. "Connect" is disabled.

## Final state (output)

- Nothing can be written for an agent that hasn't run here.

## Check on Linux

- [ ] Same five agents, the same detection by folder, and Connect disabled for a missing one?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
