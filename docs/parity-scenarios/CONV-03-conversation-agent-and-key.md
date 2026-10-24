# CONV-03: Choose the Agent that answers, and store an Anthropic API key

Category: Conversation tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/ConversationView.swift` (`agentSetting`, `saveKey`, `apply`)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`setResidentBrain`, `setAnthropicKey`)
- `app/Sources/VibeBuddyCore/Resident.swift` (`ResidentBrainChoice`)

Daemon API: `PUT /v1/resident/brain {"brain"}`; `PUT /v1/resident/anthropic-key`; `DELETE /v1/resident/anthropic-key`

## Initial state (input)

- The Conversation tab is shown and the daemon reports `brain_setting`; an older daemon doesn't, and then there is only Codex and no setting.

## Trigger

"Answered by" → Automatic / Codex / Claude Code, or the key field.

## Workflow (Mac app)

1. The segmented picker sends the choice; the daemon answers with its info, which becomes the tab's state.
2. Automatic adds "Automatic: Codex when it's ready, else Claude Code with your API key."
3. For Automatic and Claude Code, the key row: a secure field "Anthropic API key" with Save (disabled while blank), or, once the daemon has one, "Anthropic API key saved" with Remove. The key is never shown again; the daemon only says whether it has one. Saving clears the field once the daemon confirms.
4. A "Get an API key" link opens the Claude Console, and a note says Claude Code answers only with the user's own key, billed to their Anthropic account, not a Claude subscription, and that what they say and their Agents' task titles go to Anthropic.
5. A setting that fails shows its error in red under the setting.

## Final state (output)

- The daemon answers voice commands with the chosen Agent.

## Check on Linux

- [ ] Same three choices, key handling (never shown back, Remove), link and note?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
