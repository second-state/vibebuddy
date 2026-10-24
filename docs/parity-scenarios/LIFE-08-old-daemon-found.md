# LIFE-08: An old daemon is found at launch

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`LegacyLaunchAgent.migrateIfNeeded`, `portOccupied`)

Daemon API: `GET /v1/status (1 s timeout)`; `POST /v1/daemon/restart`

## Initial state (input)

- Either `~/Library/LaunchAgents/com.agentbeacon.beacond.plist` exists, or a daemon already answers `GET /v1/status` with 200 within 1 s (for example a `cargo run`).

## Trigger

The app starts.

## Workflow (Mac app)

1. An alert explains that the app now manages its own daemon and two would fight over the serial port. Title: "Found an old vibebuddyd background service" (plist found) or "A daemon is already running on port 7331". Buttons: "Remove and take over", "Later".
2. Remove and take over: `launchctl bootout gui/<uid>/com.agentbeacon.beacond`, the plist goes to the Trash, a daemon on the port is asked to exit with `POST /v1/daemon/restart`, and the app starts and manages its own daemon.
3. Later: the app starts no daemon this time and talks to the one already running; Restart then goes over HTTP (LIFE-07).

## Final state (output)

- Either the app owns the only daemon, or it is a client of the existing one for this run.

## Check on Linux

- [ ] What does the Linux app do when the daemon it talks to isn't the systemd service (started by hand)? Restart, the daemon log and the failure alert should not act on the unit as if it were.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
