# ACCT-01: Sign in to the VibeBuddy account through the browser

Category: Account tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`AccountView.signIn`)
- `app/Sources/VibeBuddy/DaemonClient.swift` (`signIn`)
- `app/Sources/VibeBuddyCore/Status.swift` (`AccountStatus`)

Daemon API: `POST /v1/account/sign-in (200 with a `url`)`; `GET /v1/status/stream (`account`)`

## Initial state (input)

- The daemon runs, its build can sign in (`account.available`), and nobody is signed in.

## Trigger

Settings → Account → "Sign in".

## Workflow (Mac app)

1. The tab says "Signing in opens your browser. Voice commands need an account."
2. The app asks the daemon for a sign-in URL and opens it in the browser; next to the button: "Waiting for the browser…".
3. Each click starts a new sign-in; the daemon drops any unfinished one.
4. The person signs in in the browser. The daemon keeps the session; the app only learns of it from the status stream (`account.signed_in`), which clears the waiting text.
5. Signed in, the tab reads "Signed in with GitHub as {login}", or "Signed in with GitHub (user {id})" when the login lookup failed, or "Signed in as {email}".
6. A failure shows "Couldn't sign in: {reason}" and clears the waiting text.

## Final state (output)

- The daemon holds the account session; the tab offers Sign out and Delete account… (ACCT-02).

## Check on Linux

- [ ] Same flow: the daemon makes the URL, the browser opens, the status stream ends the wait, and the same signed-in texts?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
