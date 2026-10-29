# LIFE-06: The daemon fails three times in a row and the app gives up

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/DaemonSupervisor.swift` (`handleExit`)
- `app/Sources/VibeBuddy/AppModel.swift` (`onGiveUp`, `notifyLink`)
- `app/Sources/VibeBuddyCore/MenuState.swift`

## Initial state (input)

- The app owns its daemon, and the daemon keeps failing to start or crashing.

## Trigger

A fourth failure in a row (more than the three backoff delays).

## Workflow (Mac app)

1. The supervisor's state becomes `givenUp` and it stops relaunching.
2. The app marks the daemon not alive and shows the daemon-down menu: "daemon not responding · click to restart", clickable.
3. If the last status the daemon reported had "Notify me when the box disconnects or the daemon fails" on (on if none was ever reported), a notification is posted: title "VibeBuddy", body "The daemon failed to start three times in a row. Click the menu bar icon to restart it."

## Final state (output)

- No daemon runs until the user restarts it (LIFE-07).
- At most one notification for this run of failures.

## Check on Linux

- [ ] Known difference: Linux notifies once after 30 s without the daemon (`outage.rs`), with its own wording.
- [ ] Does Linux follow the same toggle rule: the last reported setting, on before any status?
- [ ] Does the tray's device line offer the restart while the daemon is down?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
