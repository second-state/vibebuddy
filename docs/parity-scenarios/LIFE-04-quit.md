# LIFE-04: Quit: the app stops its daemon and the box goes offline

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`quit`, SIGTERM handler, `applicationWillTerminate`)
- `app/Sources/VibeBuddy/AppModel.swift` (`shutdown`)
- `app/Sources/VibeBuddy/DaemonSupervisor.swift` (`stop`)

## Initial state (input)

- The app runs and owns its daemon; the box is online.

## Trigger

"Quit VibeBuddy (the box goes offline)" from the menu bar or the app menu, ⌘Q, or a SIGTERM (`kill`, `pkill`).

## Workflow (Mac app)

1. SIGTERM is caught and turned into a normal quit, so the next steps still run.
2. `applicationWillTerminate` → `AppModel.shutdown`: the status stream is cancelled and the supervisor terminates the daemon and waits for it.
3. With no daemon, the box stops getting heartbeats and goes offline.
4. If the app is killed outright (SIGKILL), the daemon notices its parent pid is gone and exits within about 2 s.

## Final state (output)

- No app and no daemon; the box shows it is offline.

## Check on Linux

- [ ] Known difference, by design: on Linux Quit leaves the daemon running and the box online (platform-parity.md). Check that the tray's Quit says so, and that the box stays online.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
