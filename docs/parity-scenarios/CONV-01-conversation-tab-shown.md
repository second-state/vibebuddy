# CONV-01: The Conversation tab appears only while the daemon runs the resident session

Category: Conversation tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppModel.swift` (`residentEnabled`, `refreshResident`, `apply`)
- `app/Sources/VibeBuddy/SettingsView.swift` (`SettingsView`)
- `app/Sources/VibeBuddyCore/Resident.swift` (`ResidentInfo`)

Daemon API: `GET /v1/resident`

## Initial state (input)

- The daemon was started with `VIBEBUDDY_RESIDENT_SESSION=1` (experimental), or without it.

## Trigger

The daemon comes up: the first status snapshot after it was down.

## Workflow (Mac app)

1. The app asks `GET /v1/resident` once each time the daemon comes up, since only the daemon's environment at start decides.
2. `enabled` true: the Conversation tab is shown between Device and Account. False: no tab.
3. The answer is kept while the daemon is down, so an open tab can say the daemon isn't running (CONV-02).

## Final state (output)

- The tab is there exactly when voice commands are switched on in the daemon.

## Check on Linux

- [ ] Is the Conversation tab shown under the same rule, asked once each time the daemon comes up? Does Linux move off the tab when it goes away?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
