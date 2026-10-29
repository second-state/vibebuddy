# AGENT-06: At every launch the hook binary is refreshed and stale hook commands are fixed

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/HookInstaller.swift` (`deployBinary`, `migrateLegacyCommands`)
- `app/Sources/VibeBuddy/AppModel.swift` (`start`)

## Initial state (input)

- The app starts, possibly after an upgrade.

## Trigger

App launch.

## Workflow (Mac app)

1. The bundled `vibebuddy-hook` replaces the installed copy, so the hook updates with the app; a leftover `beacon-hook` is deleted.
2. Claude Code's and Codex's configs are scanned for VibeBuddy commands that aren't the current one (old path, old name or the old Python scripts). Each such config is rewritten with the install plan, without asking.
3. If Codex's was rewritten, the user is told to re-trust it (NOTIF-03).

## Final state (output)

- Hooks run the app's current binary.

## Check on Linux

- [ ] Known difference: no migration on Linux; `install.sh` installs the binary. Does an upgrade there leave hooks pointing at a valid binary?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
