# ADV-01: Advanced: the daemon's status and the config path

Category: Advanced tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AdvancedView`)
- `app/Sources/VibeBuddyCore/Status.swift` (`DaemonInfo.shortBuild`)

## Initial state (input)

- Settings → Advanced is open.

## Trigger

Any status change.

## Workflow (Mac app)

1. The daemon row reads "Running · {revision, or the whole build line from an older daemon}" while alive, otherwise "Not running".
2. The caption reads "Config file: {path}", the full path of `~/Library/Application Support/VibeBuddy/config.json`.

## Final state (output)

- The user can see whether the daemon runs and which build.

## Check on Linux

- [ ] Same row and config path line (with the XDG path)?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
