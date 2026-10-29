# AGENT-07: Allow K2 to reach a terminal's exact tab (Automation access)

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`TerminalAccessRow`)
- `app/Sources/VibeBuddy/TerminalAccess.swift`
- `app/Sources/VibeBuddy/AppModel.swift` (`refreshTerminalAccess`, `allowTerminalAccess`)

## Initial state (input)

- Ghostty, iTerm2 or Terminal is installed.

## Trigger

The Agents tab is shown, the app becomes active again, or the user clicks a row's button.

## Workflow (Mac app)

1. Each installed supported terminal gets a row; its state is read without asking: Allowed (green), "Not allowed yet" (grey), "Denied. Turn VibeBuddy back on for {terminal} under Privacy & Security → Automation." (red), or "Not checked yet: macOS only answers while {terminal} is running" (grey).
2. "Allow…" starts the terminal in the background if needed (waiting up to 5 s), then shows macOS's own prompt.
3. "Open System Settings" opens Privacy & Security → Automation.
4. Asking here keeps the prompt from appearing the first time K2 is pressed, away from the computer.

## Final state (output)

- With access, K2 goes to the session's own tab, not just its terminal.

## Check on Linux

- [ ] Known difference, by design: Linux has no terminal tabs or Automation permission; K2 focuses the terminal window (platform-parity.md). Nothing to check beyond the documentation.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
