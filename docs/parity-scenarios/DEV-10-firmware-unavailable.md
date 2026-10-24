# DEV-10: No firmware to flash yet, and what Latest firmware says

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`FirmwareUnavailable`, `DeviceView.latestFirmware`)

Daemon API: `POST /v1/updates/check (Check again)`

## Initial state (input)

- Firmware is wanted (a foreign box, or newer firmware exists) but isn't on disk.

## Trigger

Settings → Device or the guide.

## Workflow (Mac app)

1. Update checks off: "Update checks are off, so VibeBuddy can't download firmware."
2. On, with an error: "Couldn't get the firmware: {error}"; an offer still downloading: "Downloading the firmware…"; nothing found yet: "Looking for firmware…". Each with "Check again".
3. Always: "Or download the firmware zip yourself and use Flash from file… on the Device tab:" and a Releases link.
4. The Latest firmware line reads the offered version, or "—" (no update info), "Update checks are off (General)", "Last check failed", "Not checked yet", "None released yet".

## Final state (output)

- The user knows why nothing can be flashed and has a manual way.

## Check on Linux

- [ ] Same states and texts, and the Releases link?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
