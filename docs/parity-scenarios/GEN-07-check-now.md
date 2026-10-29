# GEN-07: Check for updates now

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`checkForUpdates`)
- `app/Sources/VibeBuddy/Updater.swift` (`checkForUpdates`)

Daemon API: `POST /v1/updates/check`

## Initial state (input)

- Update checks are on (otherwise "Check now" is disabled).

## Trigger

General → "Check now", or the menu's Check for Updates….

## Workflow (Mac app)

1. The app asks the daemon to read the manifest now; a non-202 answer becomes an alert ("Update check was rejected ({code})" or the daemon's message).
2. Sparkle shows its own window for the app: what's new, install, remind later or skip.
3. The daemon's answer arrives in later status snapshots and updates the summary and the Device tab.

## Final state (output)

- The summary shows the fresh result; a newer firmware starts downloading.

## Check on Linux

- [ ] Does Check now trigger the daemon check and show the outcome in the same summary?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
