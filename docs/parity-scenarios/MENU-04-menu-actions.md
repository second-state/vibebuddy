# MENU-04: Menu bar actions: Settings, Check for Updates, Report a Problem, Quit

Category: Menu bar · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`render`, `showSettings`)
- `app/Sources/VibeBuddy/AppModel.swift` (`checkForUpdates`, `reportProblem`)

Daemon API: `POST /v1/updates/check`

## Initial state (input)

- The app is running.

## Trigger

The user picks an item.

## Workflow (Mac app)

1. Settings… (⌘,): opens or brings forward the Settings window (760 × 640, resizable down to 640 × 480).
2. Check for Updates…: asks the daemon to read the update manifest now and shows Sparkle's own update window (GEN-07).
3. Report a Problem…: opens a new GitHub issue in the browser with the versions filled in (GEN-10).
4. Quit VibeBuddy (the box goes offline): LIFE-04.

## Final state (output)

- Each item does its one job.

## Check on Linux

- [ ] Same four items? Known difference: on Linux Check for Updates opens General, where the answer appears, since there is no Sparkle window.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
