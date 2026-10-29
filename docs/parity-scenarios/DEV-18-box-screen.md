# DEV-18: See and save what the box's screen shows

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`takeScreenshot`, `saveScreenshot`)

Daemon API: `POST /v1/device/screenshot (200 with a PNG)`

## Initial state (input)

- The box is connected; no operation runs.

## Trigger

Box screen → "Refresh", then "Save image".

## Workflow (Mac app)

1. Before any picture: "Click Refresh to see what the box is showing" on a dark panel.
2. Refresh shows a spinner while busy (Refresh disabled), then the picture at one point per pixel (320 × 240), without smoothing.
3. A failure shows the daemon's text in an alert.
4. Save image (enabled once there is a picture) opens a save panel proposing `vibe-buddy-<unix time>.png` and writes a PNG.

## Final state (output)

- The user sees the box's screen and can keep a copy.

## Check on Linux

- [ ] Known difference: Linux saves to the Pictures folder without a panel. Same Refresh states and failure handling?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
