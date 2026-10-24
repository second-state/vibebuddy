# DEV-04: The box isn't found: the pin notice and the help

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`PinNotice`, `BoxNotFoundHelp`, `BoxSearchHelp`)

## Initial state (input)

- The daemon is alive, nothing is linked, no operation runs and no flash waits for a replug.

## Trigger

Settings → Device (or the guide's Find the box).

## Workflow (Mac app)

1. If `device.pin` is set: an orange "Only looking at {value}, set by {variable}." and "To find the box on its own again, run `launchctl unsetenv <variable>` in Terminal, then restart VibeBuddy." It is shown even while connected, since a pin hides every other box.
2. With no candidates and no pin: "Not showing up? Use a cable that carries data, not just power, and click Allow if macOS asks whether to let the accessory connect." then "Still nothing? Hold K0 on the box while you plug in the cable, then click Allow. The box starts in download mode with a dark screen, ready to be flashed with VibeBuddy firmware."

## Final state (output)

- The user knows the usual causes and the download-mode last resort.

## Check on Linux

- [ ] Known difference: Linux's help points at "Permission denied" in the daemon log (the serial group) instead of macOS's accessory prompt. Is the pin notice shown with a Linux way to clear it?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
