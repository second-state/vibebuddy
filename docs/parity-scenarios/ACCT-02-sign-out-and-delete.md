# ACCT-02: Sign out, or delete the account

Category: Account tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AccountView.signOut`, `confirmDelete`)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`signOut`, `deleteAccount`, `accountCall`)

Daemon API: `POST /v1/account/sign-out`; `DELETE /v1/account (30 s timeout: the daemon gives the service 20 s)`

## Initial state (input)

- Someone is signed in.

## Trigger

"Sign out", or "Delete account…".

## Workflow (Mac app)

1. Sign out asks the daemon to end the session; a failure shows "Couldn't sign out: {reason}".
2. Delete account… first asks, as a warning, "Delete your VibeBuddy account?": "This deletes the account and everything kept about it, and signs out on every computer. It can't be undone." with a destructive "Delete account" and Cancel.
3. Confirmed, the daemon deletes the account on the service; when the service doesn't, the daemon stays signed in, and the app shows "Couldn't delete the account: {reason}".
4. The status stream then reports signed out and the tab offers Sign in again.

## Final state (output)

- Signed out on this computer, or the account deleted and signed out everywhere.

## Check on Linux

- [ ] Same confirmation text and destructive button, and the same failure handling? Does the Linux app also wait longer than usual for these calls?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
