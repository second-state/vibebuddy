# DEV-06: Firmware actions need the cable, unless the firmware updates over the link

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView.canFlash`)

## Initial state (input)

- The box is linked over Wi-Fi or the Relay (`device.network` true), and an update is available.

## Trigger

Settings → Device.

## Workflow (Mac app)

1. The box can be flashed when it is connected and either on the cable or running firmware that reports `TRANSFER 1` (`device.transfer`, ADR-0016).
2. Firmware that can't: instead of Update, "Plug in the cable to update the box's firmware."; "Flash from file…" and Retry are disabled, since the daemon would refuse a flash over the network.
3. Firmware that can: Update, Flash from file… and Retry work over Wi-Fi or the Relay as on the cable.

## Final state (output)

- Older firmware is flashed only on the cable; firmware that takes updates over the link is updated wherever the box is.

## Check on Linux

- [ ] Same rule, from the same `transfer` field?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
