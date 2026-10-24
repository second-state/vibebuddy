# MENU-03: Menu bar while the daemon runs but no box is linked

Category: Menu bar · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddyCore/MenuState.swift` (`derive`)

## Initial state (input)

- The daemon is alive and `device.connected` is false.

## Trigger

A status snapshot arrives.

## Workflow (Mac app)

1. Icon: eyes closed, greyed (offline).
2. Device line: "Box not found", disabled.
3. Mode and Today lines are still shown from the status.

## Final state (output)

- The user can tell "no box" from "no daemon": only the latter offers a restart.

## Check on Linux

- [ ] Does the tray show an offline icon distinct from daemon-down, with "Box not found" and no restart action?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
