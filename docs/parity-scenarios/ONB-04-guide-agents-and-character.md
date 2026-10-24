# ONB-04: First-run guide: Connect agents and Pick a character

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift` (`hooks`, `voices`, `voiceFootnote`)

Daemon API: `POST /v1/device/voice-pack`

## Initial state (input)

- The guide is on Connect agents or Pick a character.

## Trigger

The user connects an agent or uses a Character.

## Workflow (Mac app)

1. Connect agents shows the same rows and plan sheet as the Agents tab (AGENT-02 to AGENT-05) and says only session IDs, event names and working directories are forwarded.
2. Pick a character shows the robot and the bundled Characters (no custom card), the voice-pack progress row, and a footnote: in a Chinese UI with a Chinese Character bundled, "The box's built-in voice speaks English (Jessica); pick a Chinese voice above to hear announcements in Chinese.", otherwise "If you skip this, the box uses its built-in voice, Jessica (English)."

## Final state (output)

- Agents connected and a Character on the box, or skipped.

## Check on Linux

- [ ] Known difference: Linux has no agents page (install.sh connects them). Does Pick a character show the same cards and footnote rule?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
