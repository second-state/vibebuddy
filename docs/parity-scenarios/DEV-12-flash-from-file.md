# DEV-12: Flash a firmware zip chosen by the user

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView.flashFromFile`)
- `app/Sources/VibeBuddy/AppModel.swift` (`openFirmwarePackage`, `flashFirmware`)
- `app/Sources/VibeBuddyCore/FirmwarePackage.swift` (`inspect`)

Daemon API: `POST /v1/device/firmware`

## Initial state (input)

- The box can be flashed (DEV-06); no operation runs.

## Trigger

"Flash from file…".

## Workflow (Mac app)

1. An open panel "Choose a firmware package" accepts one zip.
2. The zip is unpacked into a fresh temporary folder (kept until the flash ends, since the daemon reads the files by path); a failure says "Couldn't unzip {name}".
3. The three images are found anywhere inside, and checked: the bootloader starts with 0xE9, the partition table with 0xAA 0x50, the app has the app-description magic at 0x20. Errors: "The firmware package has no {file}", "{file} is not an ESP32-S3 image".
4. The build comes from `build.txt`, else the version string in the image; the version from `version.txt`, if any.
5. An alert "Flash this firmware?" shows "Firmware package: {version · build}", "Box now: {version · build or unknown}" and the restart note. Flash or Cancel.
6. Flashing then runs as in DEV-05.

## Final state (output)

- The box runs the chosen firmware.

## Check on Linux

- [ ] Same checks, error texts and confirmation? (Linux: `firmware_file.rs`.)

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
