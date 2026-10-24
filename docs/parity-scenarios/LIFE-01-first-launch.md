# LIFE-01: First launch: the daemon starts and the first-run guide opens

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`applicationDidFinishLaunching`)
- `app/Sources/VibeBuddy/AppModel.swift` (`start`)
- `app/Sources/VibeBuddy/DaemonSupervisor.swift` (`launch`)
- `app/Sources/VibeBuddy/HookInstaller.swift` (`deployBinary`)

Daemon API: `GET /v1/status/stream`

## Initial state (input)

- The app has never finished its first-run guide (`onboardingDone` is not set in its defaults).
- No other copy of the app is running, no old LaunchAgent plist exists, and nothing answers on 127.0.0.1:7331.
- The box may or may not be plugged in.

## Trigger

The user opens VibeBuddy.app.

## Workflow (Mac app)

1. Single-instance check: no other copy with the same bundle id, so the launch continues.
2. Bundle-name check: the bundle already has its current name, so nothing moves.
3. The menu bar item appears with the buddy's face, eyes closed and greyed; the app menu (Settings…, Hide, Quit, Edit, Window) is installed for when a window is open.
4. Legacy check: no old LaunchAgent plist and port 7331 is free, so nothing is asked.
5. `AppModel.start`: a pending voice from a language-change restart is read and cleared; legacy AgentBeacon folders are moved; the bundled `vibebuddy-hook` is copied to `~/Library/Application Support/VibeBuddy/bin/vibebuddy-hook` (mode 0755); hook commands with an old path are rewritten (AGENT-06); hook states are read.
6. The supervisor starts the bundled `vibebuddyd` with `VIBEBUDDY_APP_VERSION` (the app's version) and `VIBEBUDDY_PARENT_PID` (the app's pid), its stdout and stderr appended to `~/Library/Logs/VibeBuddy/vibebuddyd.log`.
7. The app follows `/v1/status/stream` and starts a 5-second timer for the box-gone check (NOTIF-01).
8. Because `onboardingDone` is unset, the "Welcome to VibeBuddy" window (560 × 540) opens, with a Dock icon while it is open.

## Final state (output)

- One daemon runs, started and owned by the app.
- The menu bar reflects the first status snapshot once it arrives (MENU-01 to MENU-03).
- The first-run guide is open on Welcome (ONB-01).

## Check on Linux

- [ ] Does the first launch open the first-run guide, and only the first launch?
- [ ] Is the daemon running and reachable when the guide opens? (Known difference: on Linux systemd starts it, not the app.)
- [ ] Is the hook binary in place before the user connects agents?
- [ ] Known difference: the Linux guide leaves out the agent and launch-at-login pages, which `install.sh` already handled (platform-parity.md).

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
