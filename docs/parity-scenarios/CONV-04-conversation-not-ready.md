# CONV-04: Voice commands aren't ready: say what's missing

Category: Conversation tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/ConversationView.swift` (`content`)
- `app/Sources/VibeBuddyCore/Resident.swift` (`ResidentProblem`)
- `app/Sources/VibeBuddy/Resources.swift` (`codexInstallPage`, `claudeInstallPage`)

Daemon API: `GET /v1/resident (`problem`)`

## Initial state (input)

- The resident session is on, but `problem` is set.

## Trigger

The tab is shown, or the daemon's `problem` changes.

## Workflow (Mac app)

1. `codex_missing`: "Voice needs Codex": "Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute." with a link "How to install the Codex CLI".
2. `codex_logged_out`: "Voice needs Codex": "Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute."
3. `claude_missing`: "Voice needs Claude Code": "Voice commands are answered by Claude Code on this computer, and it isn't installed. Install Claude Code; you don't need to sign in to it. This tab notices within half a minute." with "How to install Claude Code".
4. `anthropic_key_missing`: "Voice needs Claude Code": "Claude Code answers voice commands only with your own Anthropic API key. Add your key above."
5. Anything else: "The buddy's session isn't available: {reason}".
6. On Automatic with Codex missing or signed out, it adds "Or let Claude Code answer, with your own Anthropic API key: add it above."
7. When the problem clears, the history is read at once (CONV-02).

## Final state (output)

- The user knows what to install or set, and the tab recovers by itself.

## Check on Linux

- [ ] Same problem codes, texts and links, and the same recovery?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
