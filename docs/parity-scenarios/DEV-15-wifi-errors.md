# DEV-15: The box can't join the network

Category: Device tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`WifiSection.wifiFailure`)

## Initial state (input)

- A network was set and the box reported `device.wifi_error`.

## Trigger

The error arrives in the status.

## Workflow (Mac app)

1. The status turns red with one of: `wrong_password` "The network turned the box away; the password is probably wrong. Check it, then join again."; `not_found` "The box can't find this network. It only joins 2.4 GHz Wi-Fi: check the name, and that the box is in range."; `security` "The box can't join this network's kind of security. It joins WPA2 and open networks."; `password` "The password must be 8 to 63 characters."; `ssid` "The network name is too long: 32 bytes at most."; anything else "Couldn't join ({code}). Check the network name and password, then join again."
2. The fields stay open so the user can try again, with Forget network beside Join.

## Final state (output)

- The user knows why, and can join again.

## Check on Linux

- [ ] Same mapping from error codes to texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
