# AGENT-03: Connect OpenCode, GitHub Copilot CLI or Pi (a file of VibeBuddy's own)

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/HookInstaller.swift` (`ownedFileContents`, `ownedFilePlan`, `apply`)
- `app/Sources/VibeBuddyCore/HookConfig.swift` (`describeOwnedFile`)

## Initial state (input)

- The agent is present and not connected.

## Trigger

"Connect".

## Workflow (Mac app)

1. The bundled hook binary is asked for the file (`vibebuddy-hook agent-file <agent> <installed binary path>`). If it can't make one, the plan is empty and Write is disabled; it never reads as "delete".
2. The file goes to `~/.config/opencode/plugins/vibebuddy.js`, `~/.copilot/hooks/vibebuddy.json` or `~/.pi/agent/extensions/vibebuddy.js`.
3. The sheet shows the whole file as added lines; Write creates the folder if needed and writes it.
4. Connected means the file mentions the installed hook binary's path.

## Final state (output)

- The agent loads VibeBuddy's plugin or hook file at its next start.

## Check on Linux

- [ ] Same three files, same paths under the user's config, same plan-then-write?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
