# DEV-08: Firmware written, but the box didn't start it

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`OperationRow`, `DeviceView`)

## Initial state (input)

- A flash finished writing; the box was in download mode by hand, so it ignores the reset.

## Trigger

`operation.state` becomes `replug`.

## Workflow (Mac app)

1. The row shows an orange warning: "Firmware flashed, but the box didn't start it. Unplug the box and plug it back in."
2. The box-search help stays hidden while this waits.

## Final state (output)

- After a replug the box starts the new firmware and is found.

## Check on Linux

- [ ] Same replug state and text?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
