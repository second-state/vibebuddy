# NOTIF-01: Notify once when the box has been gone for 30 seconds

Category: Notifications · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`checkLink`)
- `app/Sources/VibeBuddy/Notifier.swift`

## Initial state (input)

- The daemon is alive, "Notify me…" is on, and the box was online.

## Trigger

The box disconnects (cable out, power off, link lost).

## Workflow (Mac app)

1. Every 5 s the app checks the last status. While `device.connected` is false it remembers when the box was first missed.
2. After 30 s it posts one notification: title "Box disconnected", body "VibeBuddy hasn't seen the box for 30 seconds. Check the USB cable."
3. When the box is back, the timer and the record that it already notified both reset, so the next outage notifies again.
4. With the toggle off or the daemon down, the timer is cleared and nothing is posted.
5. The first notification asks macOS for permission; if the user refuses, nothing shows.

## Final state (output)

- At most one notification per disconnection lasting 30 s or more.

## Check on Linux

- [ ] Known difference: on Linux the daemon sends this one through `notify-send` (`link_alert.rs`). Same 30 s rule, once per disconnection, same toggle?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
