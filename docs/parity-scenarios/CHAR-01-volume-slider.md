# CHAR-01: Set the box's volume

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`VolumeRow`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setVolume`)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`setVolume`)

Daemon API: `POST /v1/device/volume {"level": n, "preview": false}`

## Initial state (input)

- The box is connected and no voice-pack write or flash is running.

## Trigger

The user drags the Volume slider and lets go.

## Workflow (Mac app)

1. The slider runs from 20 to 100 in steps of 5; the floor keeps a saved zero from becoming a permanent mute (muting is box-only: long-press K2).
2. While dragging, status snapshots don't move the slider; on release the app sends the level.
3. The box applies it and reports a `VOLUME` line; the new `device.volume` arrives in the status stream and the slider follows it.
4. The row is disabled while the box isn't connected or an operation runs. The caption says the level is saved on the box and doesn't affect previews on the computer.

## Final state (output)

- `device.volume` is the chosen level, kept by the box across restarts.

## Check on Linux

- [ ] Same range, step, floor and send-on-release; does the slider follow the box's reported volume rather than its own value?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
