# CHAR-04: Write a Character to the box

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`VoiceCard`, `OperationRow`, `VoicesView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`writeVoice`, `addressed`)

Daemon API: `POST /v1/device/voice-pack?address={form} (application/octet-stream)`

## Initial state (input)

- The box is connected and no operation runs (otherwise Use is disabled).

## Trigger

"Use" on a Character card.

## Workflow (Mac app)

1. The app loads the bundled pack; if a form of address is picked for the Character's language and a variant exists, it swaps in the addressed lines (CHAR-07).
2. It posts the pack, with the form of address as an `address` query parameter when one is picked; the daemon keeps it for the buddy's spoken replies. The daemon starts a `voice_pack` operation.
3. The tab shows "Writing voice pack… {n}%" with a progress bar, from `operation.progress`.
4. On success: "Voice pack written"; the box says a line as the new Character; `device.voice` becomes its id and its card shows "In use" with a highlighted border.
5. The header text names the current Character ("VibeBuddy" for the robot or built-in voice, "Your own character" for a custom one).

## Final state (output)

- The box speaks as the chosen Character; the choice survives box restarts.

## Check on Linux

- [ ] Same flow, progress text, "In use" marking and current-name line? Is the form of address applied the same way?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
