# DEV-05: Update the box's firmware over the cable

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`, `FlashConfirm.update`, `OperationRow`)
- `app/Sources/VibeBuddy/AppModel.swift` (`updateFirmware`)
- `app/Sources/VibeBuddyCore/Firmware.swift` (`updateAvailable`, `files`, `notes`)

Daemon API: `POST /v1/device/firmware {"bootloader", "partition_table", "app"} (file paths)`

## Initial state (input)

- The box runs our firmware and can be flashed: on the cable, or linked any way when its firmware takes updates over the link (`device.transfer`, ADR-0016; DEV-06).
- `updates.firmware` is newer than the box and downloaded (`directory` set).

## Trigger

"Update to {version}".

## Workflow (Mac app)

1. An alert "Update the box firmware to {version}?" shows the release notes in the UI language (English otherwise), then "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes." Buttons Update and Cancel.
2. Update posts the three image paths from the download folder (`bootloader.bin`, `partition-table.bin`, `vibebuddy-fw.bin`). The daemon sends them over the cable, or over whatever carries the link for firmware that takes updates that way.
3. The row shows "Flashing firmware… {n}%"; the button is disabled while it runs.
4. Done: "Firmware flashed, the box has restarted"; the Box firmware line shows the new version.

## Final state (output)

- The box runs the new firmware; voice pack and today's stats are kept.

## Check on Linux

- [ ] Same offer conditions, confirmation with notes, progress and result texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
