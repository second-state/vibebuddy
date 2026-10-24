# GEN-08: A newer app version is offered

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`installApp`)

## Initial state (input)

- `updates.app` names a newer version.

## Trigger

General is open.

## Workflow (Mac app)

1. The summary says "VibeBuddy {version} is available".
2. With Sparkle in this build: "Install {version}…" opens Sparkle's window. Without: "Download {version}" opens the offer's URL in the browser.

## Final state (output)

- The user can install or download the new app.

## Check on Linux

- [ ] Known difference, by design: Linux says how to upgrade (rerun `install.sh`) and offers the Releases page.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
