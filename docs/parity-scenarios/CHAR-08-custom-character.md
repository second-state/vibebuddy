# CHAR-08: Make and use your own Character

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`CustomCharacterCard`)
- `app/Sources/VibeBuddy/AppModel.swift` (`writeCustomCharacter`, `savedCustomCharacter`)
- `app/Sources/VibeBuddyCore/Look.swift` (`LookBuilder`)

Daemon API: `POST /v1/device/voice-pack`

## Initial state (input)

- The user has one to four PNG or JPEG drawings on a plain white or transparent background.

## Trigger

"Your own character" card → "Choose images…".

## Workflow (Mac app)

1. Files are sorted by name (natural order), so normal, eyes closed, happy, sad come in the order meant.
2. Not one to four readable images: "Pick one to four PNG or JPEG images." No figure found: "No figure found: use a plain white or transparent background."
3. The cut-out frames are previewed; a picker "Voice and lines from" chooses which bundled Character lends its voice and lines.
4. "Use" builds the pack with the look, the lender's lines (addressed, CHAR-07) and the id `custom`; a lender that can't lend shows "That character can't lend its voice; pick another."
5. The look is saved to `custom-look.bin` and the lender to the app's defaults, then the pack is written as in CHAR-04, with `lender` and `address` query parameters: the daemon keeps them, since the pack doesn't carry who lent the voice, persona and lines.
6. "Copy a prompt" puts a ready-made image-generator prompt on the clipboard.
7. After a restart the card shows the last custom Character written.

## Final state (output)

- The box shows the user's figure and speaks with the lender's voice; the card shows "In use".

## Check on Linux

- [ ] Is there a way to bring your own images, with the same checks, lender choice, prompt and persistence? (platform-parity.md lists it under Character.)

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
