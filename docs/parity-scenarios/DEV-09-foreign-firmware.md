# DEV-09: The box runs other firmware: flash VibeBuddy

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`, `FlashConfirm.foreign`)
- `app/Sources/VibeBuddyCore/Firmware.swift` (`foreign`)

Daemon API: `POST /v1/device/firmware`

## Initial state (input)

- The box is connected, the daemon says `foreign_firmware`, and no build has been reported (a factory box, say).

## Trigger

Settings → Device (or the guide's Find the box).

## Workflow (Mac app)

1. An orange "The box isn't running VibeBuddy firmware."
2. With the firmware downloaded: "Flash VibeBuddy firmware" asks, as a warning, "Flash the box with VibeBuddy?": "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes." Flash or Cancel.
3. Without it: why not, and how to get it (DEV-10).
4. Flashing then runs as in DEV-05.

## Final state (output)

- The box runs VibeBuddy firmware.

## Check on Linux

- [ ] Same detection rule (foreign and no build), warning and flash path?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
