# LIFE-03: Opening the app again while it runs brings its window forward

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`SingleInstance.handOffToRunningCopy`, `applicationShouldHandleReopen`, `showMainWindow`)

## Initial state (input)

- A copy of the app is already running.

## Trigger

The user opens the app again: a second copy (another path, a dev build), or the same app from Finder or the Dock.

## Workflow (Mac app)

1. A second process finds the running copy by bundle id and posts the `com.vibebuddy.app.showSettings` distributed notification, yields activation to it, and exits before starting any daemon.
2. If the running copy has the old bundle name and the new one has the current name, the old copy is asked to quit (up to 10 s, which stops its daemon) and the new copy takes over instead.
3. Reopening the running app itself calls `applicationShouldHandleReopen`.
4. Either way the running copy shows the first-run guide if it is still open (including minimised), otherwise Settings, and comes to the front.

## Final state (output)

- Exactly one copy and one daemon run.
- The guide or Settings window is in front.

## Check on Linux

- [ ] Does launching the app while it runs open the settings window (or the guide while it is still open) without a second app or daemon?

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
