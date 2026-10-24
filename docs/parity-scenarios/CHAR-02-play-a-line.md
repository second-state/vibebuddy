# CHAR-02: Play a line on the box at the chosen volume

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`VolumeRow`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setVolume(preview:)`)

Daemon API: `POST /v1/device/volume {"level": n, "preview": true}`

## Initial state (input)

- The box is connected; no operation runs.

## Trigger

"Play a line on the box".

## Workflow (Mac app)

1. The app sends the slider's level with `preview: true`.
2. The box sets that volume and says a line at it.

## Final state (output)

- The user hears the box at the new volume, which is now saved.

## Check on Linux

- [ ] Same button, same request?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
