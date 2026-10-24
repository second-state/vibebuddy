# LIFE-02: Later launches: menu bar only, or Settings after a relaunch

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`applicationDidFinishLaunching`)
- `app/Sources/VibeBuddy/AppLanguage.swift` (`AppRelaunch`)

Daemon API: `GET /v1/status/stream`

## Initial state (input)

- The first-run guide was finished (`onboardingDone` is true).

## Trigger

The app starts: at login, from Finder, or relaunched by the app itself with `--show-settings` (GEN-01).

## Workflow (Mac app)

1. Everything in LIFE-01 up to following the status stream happens the same way.
2. No window opens, unless the command line carries `--show-settings`, in which case the Settings window opens.

## Final state (output)

- The app lives in the menu bar only; no Dock icon.
- After a language-change relaunch, Settings is open again where the user left off.

## Check on Linux

- [ ] Does a normal start show no window (tray only)?
- [ ] After the app restarts itself for a language change, does Settings open again?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
