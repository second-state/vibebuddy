# NOTIF-03: Tell the user when the app rewrote Codex's hook config

Category: Notifications · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`start`)
- `app/Sources/VibeBuddy/HookInstaller.swift` (`migrateLegacyCommands`)

## Initial state (input)

- `~/.codex/hooks.json` holds VibeBuddy hook commands with an old path or name.

## Trigger

The app starts.

## Workflow (Mac app)

1. The app rewrites the stale commands to the current binary (AGENT-06).
2. If Codex's config was among those rewritten, it posts, whatever the notify toggle says: title "Codex hook config updated", body "VibeBuddy rewrote ~/.codex/hooks.json. Codex silently disables hooks that change: type /hooks in Codex and re-trust them so the box keeps getting Codex events."

## Final state (output)

- The user knows to re-trust Codex's hooks.

## Check on Linux

- [ ] Known difference, by design: there is no such migration on Linux (platform-parity.md). Nothing to check unless Linux ever rewrites hook configs by itself.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
