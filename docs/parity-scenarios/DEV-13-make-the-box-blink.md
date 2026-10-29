# DEV-13: Make the box blink

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`identify`)

Daemon API: `POST /v1/device/identify`

## Initial state (input)

- The box is connected; no operation runs.

## Trigger

"Make the box blink" (or "Make it blink" in the guide).

## Workflow (Mac app)

1. The app asks the daemon to identify the box; the box flashes its backlight.

## Final state (output)

- The user can tell which device is the box.

## Check on Linux

- [ ] Same button and request?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
