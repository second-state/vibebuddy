# CHAR-06: Go back to the robot, the default Character

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`RobotCard`)
- `app/Sources/VibeBuddy/AppModel.swift` (`writeRobot`, `wearsRobot`, `robotBorrowsVoice`)

Daemon API: `POST /v1/device/voice-pack`

## Initial state (input)

- The box wears another Character; it is connected and no operation runs.

## Trigger

"Use" on the VibeBuddy card (marked Default).

## Workflow (Mac app)

1. The app writes the bundled `robot` pack: no look of its own (the box draws the robot), the built-in voice and its five fixed lines.
2. Any record that an older version let another Character lend the robot its voice is cleared.
3. The card shows "In use" when the box's voice is `robot` or `builtin` and nothing is lent.

## Final state (output)

- The box shows and speaks as the original robot.

## Check on Linux

- [ ] Is the robot offered as its own card, marked as the default, with the same "In use" rule?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
