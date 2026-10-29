# GEN-01: Change the UI language, restart, and switch the box's voice too

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView.change`)
- `app/Sources/VibeBuddy/AppLanguage.swift`
- `app/Sources/VibeBuddy/AppModel.swift` (`restart(writingVoice:)`, `apply`)
- `app/Sources/VibeBuddyCore/VoicePack.swift` (`VoiceCatalogEntry.switchSuggestion`)

Daemon API: `POST /v1/device/voice-pack`

## Initial state (input)

- The UI is in English; the box is connected, has reported its build, and wears an English Character.
- A Chinese Character is bundled.

## Trigger

General → Language → 简体中文.

## Workflow (Mac app)

1. The choice is saved at once (System removes the override).
2. Because the resolved language differs from the one shown, an alert asks "Restart VibeBuddy to change the language?" with "The box goes offline for a few seconds while the app restarts."
3. A checkbox, on by default, offers "Also switch the box's voice to {Character}"; over the UART bridge the label adds "(a few minutes over the UART bridge; keep it plugged in)". The Character is the first bundled one in the new language.
4. Restart now: the chosen voice is saved as pending, the app quits and reopens with `--show-settings`; once the restarted app sees the box report a build and no operation is running, it writes that voice (CHAR-04).
5. Later: no restart; if the checkbox was on, the voice is written right away.

## Final state (output)

- After the restart the UI is in Chinese, Settings is open, and the box wears the Chinese Character.

## Variations

- Choosing a language that resolves to the one already shown (System, when the system language matches) only saves the choice; no alert.
- The box wears the robot or an unknown voice: no checkbox, nothing is guessed.

## Check on Linux

- [ ] Does the Linux language picker offer the same restart, with the same voice-switch offer and the same conditions?
- [ ] Is the pending voice written only once the restarted app sees the box answer?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
