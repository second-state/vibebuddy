# ONB-02: First-run guide: Find the box

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift` (`findDevice`)
- `app/Sources/VibeBuddy/SettingsView.swift` (`BoxSearchHelp`, `PinNotice`, `FlashConfirm`)

Daemon API: `POST /v1/device/identify`; `POST /v1/device/firmware`

## Initial state (input)

- The guide is on Find the box.

## Trigger

The daemon starts and the box is plugged in, or not.

## Workflow (Mac app)

1. Daemon not alive yet: "Starting the daemon…".
2. Alive, no box: "No box yet — this updates once you plug it in.", the pin notice if a port is pinned, and the box-search help (DEV-03, DEV-04).
3. Box found with our firmware: "Found it: {port}", "Make it blink" ("If the backlight flashes, that's the one.").
4. Same, with a downloaded newer firmware: "Newer firmware is available: {version}" and "Update to {version}" (confirmation as in DEV-05), plus "You can also skip this and update later on the Device tab in Settings." Newer but not downloaded: "Downloading firmware {version}…".
5. Box found with other firmware: "Found a box ({port}), but it isn't running VibeBuddy firmware." with "Flash VibeBuddy firmware" (DEV-09) or why there is none to flash yet (DEV-10).

## Final state (output)

- The user knows the box is found and running current VibeBuddy firmware, or is flashing it.

## Check on Linux

- [ ] Same states and wording, in the same order of precedence?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
