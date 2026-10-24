# MENU-01: Menu bar while the daemon is down

Category: Menu bar · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddyCore/MenuState.swift` (`derive`)
- `app/Sources/VibeBuddy/AppDelegate.swift` (`render`)

## Initial state (input)

- The daemon isn't alive (not started, crashed, given up, or the stream dropped).

## Trigger

The menu state is derived again (any liveness change).

## Workflow (Mac app)

1. Icon: the face with its eyes closed, greyed out.
2. Device line: "daemon not responding · click to restart", enabled; clicking it restarts the daemon (LIFE-07).
3. Mode and Today lines: "—", disabled.
4. Then: Settings…, Check for Updates…, Report a Problem…, Quit VibeBuddy (the box goes offline).

## Final state (output)

- The user can see the daemon is down and restart it from the menu.

## Check on Linux

- [ ] Does the tray icon show the same down state, and is the device line a restart action only while the daemon is down?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
