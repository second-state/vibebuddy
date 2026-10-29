# GEN-06: Turn update checks on or off and read their summary

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView.updateSummary`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setCheckUpdates`)
- `app/Sources/VibeBuddy/Updater.swift` (`follow`)

Daemon API: `PUT /v1/config`

## Initial state (input)

- The daemon is alive.

## Trigger

The user flips "Check for updates", or the update state changes.

## Workflow (Mac app)

1. The toggle shows `config.check_updates`, else `updates.enabled`, else off; flipping it puts the config.
2. Sparkle's automatic checks follow `updates.enabled` from each status, so one switch covers the firmware and the app.
3. The summary next to Check now reads, in order: "Off" when checks are off; "Last check failed: {error}"; "VibeBuddy {version} is available"; "Not checked yet"; otherwise "Up to date · checked {relative time}".

## Final state (output)

- Checks run or stop for both the firmware manifest and the app.

## Check on Linux

- [ ] Same toggle and the same summary texts in the same order? Known difference: no Sparkle; the app offer leads to the upgrade instructions instead.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
