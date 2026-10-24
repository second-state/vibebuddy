# GEN-02: Change the UI language while the box isn't connected

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView.change`)

## Initial state (input)

- The box is not connected.

## Trigger

General → Language → another language that changes the UI.

## Workflow (Mac app)

1. The restart alert appears without a voice checkbox, and its text adds: "The box isn't connected, so its character stays as it is. You can change it later on the Character tab."
2. Restart now or Later as in GEN-01, but no voice is written.

## Final state (output)

- The UI language changes after the restart; the box's Character is untouched.

## Check on Linux

- [ ] Same message and no voice write when the box is absent?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
