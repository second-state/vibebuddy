# CHAR-03: Preview a Character on the computer

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`VoiceCard`, `RobotCard`)
- `app/Sources/VibeBuddy/AppModel.swift` (`togglePreview`)
- `app/Sources/VibeBuddy/VoicePreview.swift`
- `app/Sources/VibeBuddyCore/VoicePack.swift` (`previewPCM`)

## Initial state (input)

- The Character's pack is bundled with the app; no operation runs (the play buttons are disabled during one).

## Trigger

The play button on a Character card.

## Workflow (Mac app)

1. The pack's preview lines are decoded to 24 kHz 16-bit stereo PCM and played on the computer.
2. The button turns into a stop button while that Character plays; pressing it, or another Character's play, stops it.
3. When playback ends the button returns to play.
4. A missing pack shows "This app has no voice pack for {id}".

## Final state (output)

- Nothing is written to the box; the box's volume is not involved.

## Check on Linux

- [ ] Same toggle behaviour? Known difference: Linux plays through `pw-play`, `paplay` or `aplay`.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
