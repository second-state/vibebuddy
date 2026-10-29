# GEN-04: Turn the disconnect and daemon-failure notifications on or off

Category: General tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`GeneralView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setNotifyLink`)

Daemon API: `PUT /v1/config`

## Initial state (input)

- The daemon is alive and has sent a status.

## Trigger

The user flips "Notify me when the box disconnects or the daemon fails".

## Workflow (Mac app)

1. The app copies the config from the last status, changes `notify_link`, and puts the whole config to the daemon.
2. The toggle follows `config.notify_link` from the status stream (on while unknown).
3. The toggle is disabled while there is no status.

## Final state (output)

- The daemon saves the setting; it governs NOTIF-01 and LIFE-06, including after the daemon goes away.

## Check on Linux

- [ ] Same toggle, same config field, disabled without a status, and honoured by both notifications?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
