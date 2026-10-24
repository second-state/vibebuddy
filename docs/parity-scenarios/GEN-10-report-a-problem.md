# GEN-10: Report a problem

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`reportProblem`, `diagnosticsSummary`)
- `app/Sources/VibeBuddyCore/IssueReport.swift`

## Initial state (input)

- Any state.

## Trigger

General → "Report a Problem…", or the menu item.

## Workflow (Mac app)

1. The app builds a summary: "App {version} ({build})", "daemon {build or not connected}", "firmware {version · build or —}", "offered firmware {version or —}", "voice {id or —}", "macOS {version}".
2. It opens `https://github.com/second-state/vibebuddy/issues/new` with a body asking "What happened?" and "What did you expect?", then the summary in a code block. `+` is encoded so GitHub doesn't read it as a space.
3. No logs and nothing an agent did go into it.

## Final state (output)

- A new issue page is open in the browser with the versions filled in.

## Check on Linux

- [ ] Same URL, body shape and summary lines (with the OS name instead of macOS)?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
