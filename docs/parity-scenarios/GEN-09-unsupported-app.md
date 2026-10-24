# GEN-09: This app version is no longer supported

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView`)

## Initial state (input)

- `updates.unsupported_app` is true.

## Trigger

General is open.

## Workflow (Mac app)

1. An orange warning at the top: "This version of VibeBuddy is no longer supported. Update it to keep getting firmware for the box."
2. Nothing else stops working.

## Final state (output)

- The warning shows every time until the app is updated.

## Check on Linux

- [ ] Same warning, in the same place?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
