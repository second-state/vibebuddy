# GEN-05: Turn the VibeBuddy Relay on or off

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setRelay`)

Daemon API: `PUT /v1/config`

## Initial state (input)

- The daemon is alive and has sent a status.

## Trigger

The user flips "Reach the box through the VibeBuddy Relay when it's away from this computer".

## Workflow (Mac app)

1. The app puts the config with `relay` changed.
2. The toggle shows `config.relay` (on while unknown) and is disabled without a status.

## Final state (output)

- The daemon uses or stops using the Relay.

## Check on Linux

- [ ] Same toggle and config field?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
