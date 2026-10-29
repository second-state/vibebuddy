# DEV-02: The box is with another computer: take it over

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`DeviceView`)
- `app/Sources/VibeBuddy/AppModel.swift` (`takeOver`)

Daemon API: `POST /v1/device/takeover`

## Initial state (input)

- This computer isn't linked; `device.busy` names the owner and a reason.

## Trigger

Settings → Device.

## Workflow (Mac app)

1. Reason `usb`: "The box is plugged into {owner}." with no button, since another computer's cable can't be taken.
2. Otherwise: "The box is with {owner}." and "Take over"; an empty owner reads "another computer".
3. Take over asks the daemon to claim the box on its next try over the network or the Relay; a refusal is an alert ("Taking the box over was rejected ({code})").

## Final state (output)

- This computer gets the box, unless another one has it on its cable.

## Check on Linux

- [ ] Same texts, same rule for when Take over is offered?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
