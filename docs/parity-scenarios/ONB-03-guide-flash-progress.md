# ONB-03: First-run guide: flashing from Find the box

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift` (`findDevice`)

Daemon API: `POST /v1/device/firmware`

## Initial state (input)

- A firmware flash was started from this page.

## Trigger

The daemon reports the `firmware` operation.

## Workflow (Mac app)

1. The flash progress is shown before any connection check, since the daemon releases the port and the box looks disconnected while it flashes: "Flashing VibeBuddy firmware…" and the progress row.
2. Running: "The box screen stays dark for a few minutes — don't unplug it. It restarts on its own when done, and this page will show it as found."
3. Failed: "Hold K0 on the box and replug the cable to put it in download mode, then retry." and Retry.
4. Replug: "Don't hold K0 this time. This page will show the box as found once it starts."
5. Done: the page falls back to the found state.

## Final state (output)

- The box runs the new firmware and the page shows it as found.

## Check on Linux

- [ ] Does the Linux guide keep showing the progress while the box looks disconnected, with the same running, failed and replug texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
