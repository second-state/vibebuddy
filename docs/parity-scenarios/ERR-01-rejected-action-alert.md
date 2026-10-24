# ERR-01: An action the daemon rejects shows an alert

Category: Errors · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`run`, `lastError`)
- `app/Sources/VibeBuddy/AppDelegate.swift` (the `lastError` alert)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`message(in:)`)

Daemon API: `Any POST that must answer 202 (200 for a screenshot)`

## Initial state (input)

- Any state.

## Trigger

A daemon call fails: wrong status code, no daemon, timeout (10 s per request).

## Workflow (Mac app)

1. The client throws the daemon's `message` from the JSON body when there is one, otherwise a fixed text such as "Volume change was rejected ({code})".
2. The model stores it as `lastError`; the app shows a modal alert titled "VibeBuddy" with that text.
3. Local failures use the same alert (an unzip that fails, a missing voice pack, a hook config that can't be written).

## Final state (output)

- The user sees why the action didn't happen; nothing else changes.

## Check on Linux

- [ ] Does Linux surface the same failures to the user (as a notice in the window), using the daemon's message when there is one?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
