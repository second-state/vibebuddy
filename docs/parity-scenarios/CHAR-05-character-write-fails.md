# CHAR-05: A Character write fails

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`VoicesView`, `OperationRow`)

Daemon API: `POST /v1/device/voice-pack`

## Initial state (input)

- A voice-pack write is running.

## Trigger

The write fails (cable out, the box refuses, a lost acknowledgement).

## Workflow (Mac app)

1. `operation.state` becomes `failed`; the row shows "Failed: {daemon message}" with the message also as its tooltip.
2. Below it: "Didn't finish, so the box keeps its built-in voice. Reconnect the cable and click Use again."
3. A request the daemon refuses outright (for example while another operation runs) is an alert instead (ERR-01).

## Final state (output)

- The box keeps its built-in voice; Use is available again once the box is connected.

## Check on Linux

- [ ] Same failure text and retry path?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
