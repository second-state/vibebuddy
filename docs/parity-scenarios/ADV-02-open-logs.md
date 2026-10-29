# ADV-02: Open the logs folder

Category: Advanced tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AdvancedView`)

## Initial state (input)

- Any state.

## Trigger

"Open logs folder".

## Workflow (Mac app)

1. Finder comes to the front with `~/Library/Logs/VibeBuddy/vibebuddyd.log` selected (explicitly brought forward, since a menu bar app's windows can open behind others).

## Final state (output)

- The daemon's log is in front of the user.

## Check on Linux

- [ ] Known difference: on Linux the folder holds the hook's log, and "Show daemon log" follows the journal in a terminal. Does each open in front?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
