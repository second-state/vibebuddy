# ONB-05: First-run guide: Wi-Fi

Category: First-run guide · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/OnboardingView.swift` (`wifi`)
- `app/Sources/VibeBuddy/SettingsView.swift` (`WifiSection`)

Daemon API: `POST /v1/device/wifi`; `POST /v1/device/wifi/scan`

## Initial state (input)

- The guide is on Wi-Fi.

## Trigger

The page is shown.

## Workflow (Mac app)

1. It explains that with Wi-Fi the computer reaches the box without the cable, over the local network or through the Relay.
2. When the box is on the cable (connected, not over the network) and has reported its box key, the Wi-Fi section from the Device tab appears (DEV-14 to DEV-16).
3. Otherwise: "Plug in the box to set its network. You can also do it later on the Device tab in Settings."

## Final state (output)

- The box has a network, or the user skipped.

## Check on Linux

- [ ] Same condition (cable and box key) and the same section?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
