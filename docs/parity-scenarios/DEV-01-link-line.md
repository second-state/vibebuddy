# DEV-01: Device: what the Link line says

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView.connectionText`)

## Initial state (input)

- Settings → Device is open.

## Trigger

Any status change.

## Workflow (Mac app)

1. No status (daemon down): "daemon isn't running".
2. Not connected, with candidates: "Several devices found"; with none: "Box not found".
3. Connected through the Relay: "Relay".
4. Connected over the local network: "{port} · Wi-Fi".
5. Connected on the cable: "{port} · UART bridge" or "{port} · native USB", from `device.bridge`.

## Final state (output)

- One line tells how, or whether, the box is linked.

## Check on Linux

- [ ] Same six cases and wording?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
