# DEV-11: Our firmware on other hardware: no update offered

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`)

## Initial state (input)

- The box is connected and its firmware named a board other than the box (`device.unsupported_board`).

## Trigger

Settings → Device.

## Workflow (Mac app)

1. A note: "This is a {board} board, which released firmware doesn't run on, so no update is offered."
2. The daemon offers no released firmware to it.

## Final state (output)

- Nobody flashes box firmware onto other hardware by accident.

## Check on Linux

- [ ] Same note?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
