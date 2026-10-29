# DEV-03: Several devices are plugged in: say which is the box

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`BoxSearchHelp`, `BoxChoice`)
- `app/Sources/VibeBuddy/AppModel.swift` (`chooseBox`)

Daemon API: `POST /v1/device/choice {"usb_serial": …}`

## Initial state (input)

- The daemon is alive, the box isn't linked, and `device.candidates` lists more than one device, none of them the box seen before.

## Trigger

Settings → Device (or the guide's Find the box).

## Workflow (Mac app)

1. "Several devices are plugged in, and none of them is the box seen before. Which one is the box?"
2. One row per candidate: "{port} · {USB serial}" (or just the port), with "This is the box" when it has a USB serial; disabled while an operation runs.
3. Choosing posts the serial; the daemon remembers it and connects to that device on its next look.

## Final state (output)

- The box is linked and remembered for next time.

## Check on Linux

- [ ] Same list and button? (platform-parity.md says Linux has "This is the box".)

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
