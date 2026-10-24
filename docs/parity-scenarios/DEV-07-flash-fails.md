# DEV-07: A firmware flash fails

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`, `OperationRow`)

Daemon API: `POST /v1/device/firmware`

## Initial state (input)

- A firmware flash is running.

## Trigger

The flash fails.

## Workflow (Mac app)

1. The row shows "Failed: {daemon message}".
2. For firmware that doesn't take updates over the link: "Before retrying, hold K0 on the box and replug the cable to put it in download mode." Firmware that does carries on from where it got to, so the hint is left out.
3. Retry, enabled when the box can be flashed (DEV-06), flashes the offered firmware again (DEV-05).

## Final state (output)

- The user can retry: from where the transfer stopped, or after putting the box in download mode.

## Check on Linux

- [ ] Same failure text, the K0 hint only for firmware without `transfer`, and Retry under the same rule?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
