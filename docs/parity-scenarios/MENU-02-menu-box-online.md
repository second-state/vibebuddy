# MENU-02: Menu bar while the box is online

Category: Menu bar · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddyCore/MenuState.swift` (`derive`, `duration`)
- `app/Sources/VibeBuddy/AppDelegate.swift` (`render`)

## Initial state (input)

- The daemon is alive and `device.connected` is true.

## Trigger

A status snapshot arrives.

## Workflow (Mac app)

1. Icon: eyes open, not greyed.
2. Device line: "Box online · firmware {first word of firmware_build, or ?}", disabled.
3. Mode line: "Mode: On duty" / "Pomodoro" / "Leisure" for `device.mode` `duty` / `pomodoro` / `leisure`, otherwise "Mode: —".
4. Today line: "Today: done {n} · asks {n} · busy {duration}", the duration as "{h} h {m} min" from an hour, otherwise "{m} min".

## Final state (output)

- The menu summarises the box and today's counts.

## Check on Linux

- [ ] Same three lines with the same values and formats in the tray menu?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
