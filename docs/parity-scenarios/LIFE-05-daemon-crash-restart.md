# LIFE-05: The daemon crashes and the app restarts it with backoff

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/DaemonSupervisor.swift` (`handleExit`, `launch`)
- `app/Sources/VibeBuddy/AppModel.swift` (`onStateChange`, `followStatus`)

Daemon API: `GET /v1/status/stream`

## Initial state (input)

- The app owns a running daemon; the menu shows the box online.

## Trigger

The daemon exits.

## Workflow (Mac app)

1. Exit code 0 means the daemon quit at the app's request (`POST /v1/daemon/restart`): it is relaunched at once and not counted as a failure.
2. Any other exit counts as a failure: the supervisor's state becomes `stopped`, so the app marks the daemon not alive, clears the status and shows the daemon-down menu (MENU-01).
3. The daemon is relaunched after 1 s, 2 s and 5 s for the first, second and third failure.
4. The status stream retries every second; the first snapshot from the new daemon marks it alive again.
5. The failure count is reset only by app start or a Restart (LIFE-07), not by a relaunched daemon that stays up.

## Final state (output)

- The daemon runs again and the menu returns to its normal state.

## Check on Linux

- [ ] Known difference: systemd restarts the daemon (`Restart=always`, `RestartSec=2`), not the app.
- [ ] While it is down, does the tray show the daemon-down state, and recover by itself once the daemon is back?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
