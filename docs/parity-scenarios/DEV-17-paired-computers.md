# DEV-17: See and remove the computers paired with the box

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`PairedComputersSection`)
- `app/Sources/VibeBuddy/AppModel.swift` (`unpair`)

Daemon API: `POST /v1/device/unpair {"key"}`

## Initial state (input)

- The box is connected and has reported its box key.

## Trigger

Settings → Device → Paired computers.

## Workflow (Mac app)

1. Linked over the network: "Plug in the cable to see the paired computers." The box lists them over the cable only, so the list would be empty.
2. One row per `device.paired_computers` entry, named, or "Unnamed computer".
3. This computer's row says "This computer" with no button: it would only pair again when plugged in.
4. Others have "Remove" (disabled during an operation); the daemon refuses this computer's key.
5. The footer: "A computer pairs with the box when it's plugged in. To forget them all, use COMPUTERS in the box's menu."

## Final state (output)

- The removed computer can no longer drive the box until it pairs again.

## Check on Linux

- [ ] Same list, "This computer" rule, Remove, and the cable note while linked over the network?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
