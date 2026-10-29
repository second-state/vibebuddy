# ACCT-03: The Account tab when the daemon is down or can't sign in

Category: Account tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AccountView`)

## Initial state (input)

- The daemon isn't running, or runs a build without an account service (`account.available` false or absent).

## Trigger

Settings → Account.

## Workflow (Mac app)

1. No status: "The daemon isn't running. Restart it on the Advanced tab."
2. A status without an available account: "Signing in isn't available in this version of VibeBuddy."

## Final state (output)

- No sign-in is offered that couldn't work.

## Check on Linux

- [ ] Same two texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
