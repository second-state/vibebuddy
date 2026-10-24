# DEV-14: Put the box on Wi-Fi

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`WifiSection`)
- `app/Sources/VibeBuddy/AppModel.swift` (`setWifi`, `scanWifi`)

Daemon API: `POST /v1/device/wifi/scan`; `POST /v1/device/wifi {"ssid", "password"}`

## Initial state (input)

- The box is on the cable and has reported its box key (the Wi-Fi section needs a connected box with a key; the fields need the cable).
- No network is set.

## Trigger

Settings → Device → Wi-Fi.

## Workflow (Mac app)

1. Network reads "Not set". The box scans as the fields appear, unless it already reported networks; "Scan" scans again.
2. "Nearby networks" lists what the box heard (`device.wifi_seen`) as "{name}  {rssi} dBm"; "Network name" can be typed for a hidden one.
3. Join is disabled for an empty name, a name over 32 bytes, a password over 63 bytes, or during an operation.
4. Join sends the name and password over the cable; the password is cleared from the field and kept nowhere else; the fields fold away.
5. Status reads "Joining… This can take up to 15 seconds.", then "Joined · {address}" in green.

## Final state (output)

- The box is on the network; a paired computer on it can drive the box without the cable.

## Variations

- Joining again, or a join that fails, on the network the box was last joined to: the daemon drops the old address meanwhile, so the status reads "Joining…" or the error, never the stale "Joined".

## Check on Linux

- [ ] Same scan, picker, limits, password handling and status texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
