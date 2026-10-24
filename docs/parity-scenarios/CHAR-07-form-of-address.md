# CHAR-07: Choose what the buddy calls you

Category: Character tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AddressRow`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setFormOfAddress`, `addressed`)
- `app/Sources/VibeBuddyCore/VoicePack.swift` (`FormOfAddress`)

Daemon API: `POST /v1/device/voice-pack (when the current Character is rewritten)`

## Initial state (input)

- Settings → Character is open.

## Trigger

"What the buddy calls you" → Chinese or English picker → a form (老板, 大佬, 哥, 姐, 亲; boss, captain, buddy) or Nothing.

## Workflow (Mac app)

1. The pick is saved per language.
2. If the box wears a bundled Character of that language (not the built-in voice), that Character is written again with the addressed lines.
3. If it wears the user's own Character whose lender has that language, the custom Character is written again (CHAR-08).
4. Otherwise nothing is written now; the pick applies the next time a Character of that language is used.
5. The pickers are disabled while an operation runs.

## Final state (output)

- The box calls the user by the chosen words in that language.

## Check on Linux

- [ ] Are forms of address offered per language, saved, and applied by rewriting the current Character?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
