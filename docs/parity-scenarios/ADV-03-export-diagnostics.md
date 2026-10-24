# ADV-03: Export diagnostics

Category: Advanced tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AdvancedView.exportDiagnostics`, `writeDiagnostics`)
- `app/Sources/VibeBuddy/AppModel.swift` (`diagnosticsSummary`)

## Initial state (input)

- Any state; the daemon may be down.

## Trigger

"Export diagnostics…".

## Workflow (Mac app)

1. A save panel (a sheet on the Settings window) proposes a folder named `vibe-buddy-diagnostics`.
2. Into it go `vibebuddyd.log` and `codex-hooks.log` if they exist, `config.json` as the daemon last reported it (the app never reads the daemon's file), and `summary.txt` (the same lines as GEN-10).
3. No hook payloads. Finder then shows the folder.

## Final state (output)

- One folder with logs, config and both sides' builds.

## Check on Linux

- [ ] Known difference: Linux writes the folder under Downloads and takes two days of the daemon's journal. Same summary and config, and nothing from agents?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
