# AGENT-05: Each agent's status dot, and the Codex trust hint

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`HookRow.dotColor`, `statusText`)
- `app/Sources/VibeBuddyCore/HookConfig.swift` (`codexTrustHint`)

## Initial state (input)

- The agent is present.

## Trigger

The Agents tab is shown; `hooks.<agent>` (last event time) arrives in each status.

## Workflow (Mac app)

1. Not connected: grey, "Not set up".
2. Connected, no event yet: orange, "Waiting for the first event…"; for Codex "Waiting for the first event… (Codex only runs this config after you trust it in /hooks)".
3. Connected with an event: green, "Last event {date and time}".
4. Codex only: when the config file changed after the last event, red: "The config changed at {time} and no Codex event has arrived since. Codex silently disables hooks that change: type /hooks in Codex and re-trust VibeBuddy's six hooks."

## Final state (output)

- The user can tell set up, working, and silently untrusted apart.

## Check on Linux

- [ ] Same colours, texts and the Codex rule (config modified time against the last event)? platform-parity.md says Linux has the Codex hint (`CodexTrust`).

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
