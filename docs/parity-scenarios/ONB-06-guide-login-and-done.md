# ONB-06: First-run guide: Launch at login and Done

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift` (`loginItem`, `done`)

## Initial state (input)

- The guide is on Launch at login.

## Trigger

The user flips the toggle and goes on.

## Workflow (Mac app)

1. The toggle works as in GEN-03; a note says macOS may report a background item.
2. Done says the app lives in the menu bar, its icon the buddy's face with eyes open while the box is online, and how to get back to Settings.

## Final state (output)

- Get started finishes the guide (ONB-01).

## Check on Linux

- [ ] Known difference: no login page on Linux (install.sh sets the autostart). Does its Done page say where the app lives (tray) and how to reopen it?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
