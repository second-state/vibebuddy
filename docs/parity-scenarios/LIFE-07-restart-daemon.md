# LIFE-07: Restart the daemon on request

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`restartDaemon`)
- `app/Sources/VibeBuddy/DaemonSupervisor.swift` (`restart`)
- `app/Sources/VibeBuddy/AppDelegate.swift` (`render`)
- `app/Sources/VibeBuddy/SettingsView.swift` (`AdvancedView`)

Daemon API: `POST /v1/daemon/restart (only when the app doesn't manage the daemon)`

## Initial state (input)

- The daemon is running, stopped or given up.

## Trigger

Advanced → "Restart daemon" (always enabled), or the menu's device line while the daemon is down.

## Workflow (Mac app)

1. When the app manages the daemon: the failure count is reset, a running daemon is terminated and waited for, and a new one is started.
2. When the user kept an old LaunchAgent (LIFE-08): the app posts `/v1/daemon/restart`; the daemon exits and the LaunchAgent starts it again. Errors are ignored.
3. The status stream reconnects and the UI follows the new daemon.

## Final state (output)

- A fresh daemon runs; the failure count starts from zero.

## Check on Linux

- [ ] Does Restart work both while the daemon runs and while it is down?
- [ ] Is a failure to restart reported to the user?
- [ ] For a daemon the app or its supervisor didn't start, does Restart say what to do instead of acting on something else?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
