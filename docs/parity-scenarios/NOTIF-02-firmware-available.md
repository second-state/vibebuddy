# NOTIF-02: Notify once per new firmware version

Category: Notifications · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`notifyNewFirmware`)
- `app/Sources/VibeBuddyCore/Firmware.swift` (`updateAvailable`)

## Initial state (input)

- The box is connected and the daemon offers firmware newer than the box's, already downloaded (`updates.firmware.newer_than_box` and `directory` set).

## Trigger

A status snapshot arrives.

## Workflow (Mac app)

1. If that version is not the one last announced (kept in the app's defaults), the app records it and posts: title "Box firmware {version} is available", body "Open Settings → Device to update the box."
2. No notification while the box is disconnected or before the download finishes; the next snapshot that meets the conditions posts it.

## Final state (output)

- One notification per version, ever, even across restarts.

## Check on Linux

- [ ] Does the Linux app post one notification per new firmware version, only while the box is connected and once the download is on disk, and remember across restarts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
