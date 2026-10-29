# GEN-03: Turn Launch at login on or off

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`setLaunchAtLogin`)
- `app/Sources/VibeBuddy/LoginItem.swift`

## Initial state (input)

- Settings → General is open.

## Trigger

The user flips "Launch at login" (also on the first-run guide's page, ONB-06).

## Workflow (Mac app)

1. The app registers or unregisters itself with `SMAppService.mainApp`; macOS may show a "background item added" notice.
2. On failure an alert says "Couldn't change Launch at login: {reason}".
3. The toggle then shows the real state read back from the system, not the requested one.

## Final state (output)

- The app starts at login, or doesn't, matching the toggle.

## Check on Linux

- [ ] Does the Linux toggle write or remove the autostart entry and show the real state afterwards, with an error when it fails? (Known difference: XDG autostart, and the pacman package's system-wide entry.)

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
