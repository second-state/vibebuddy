# DEV-16: Change or forget the box's network

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`WifiSection`)
- `app/Sources/VibeBuddy/AppModel.swift` (`forgetWifi`)

Daemon API: `POST /v1/device/wifi/forget`

## Initial state (input)

- The box has a network set with no error.

## Trigger

Settings → Device → Wi-Fi.

## Workflow (Mac app)

1. On the cable: "Change network" reopens the fields; "Forget network" asks the box to forget it (disabled during an operation).
2. Linked over the network: only "Plug in the cable to change the box's network."
3. The footer: "With Wi-Fi, a paired computer on the same network can drive the box without the cable."

## Final state (output)

- The box joins another network, or none.

## Check on Linux

- [ ] Same buttons, and the same cable-only rule?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
