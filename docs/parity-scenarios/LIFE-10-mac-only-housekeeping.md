# LIFE-10: Mac-only housekeeping: bundle rename, legacy folders, Dock icon and app menus

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/AppDelegate.swift` (`BundleRename`, `mainMenu`, `present`, `watchClose`)
- `app/Sources/VibeBuddy/Resources.swift` (`migrateLegacyDirectories`)
- `app/Sources/VibeBuddyCore/BundleName.swift`

## Initial state (input)

- Any launch.

## Trigger

The app starts, or a window opens or closes.

## Workflow (Mac app)

1. A bundle with the old name renames itself (or gives way to a copy with the new name), keeps the login item, and starts again from the new place.
2. `~/Library/Application Support/AgentBeacon` and `~/Library/Logs/AgentBeacon` move to the VibeBuddy folders if the new ones don't exist.
3. Opening a window gives the app a Dock icon and the standard menus (⌘Q, ⌘W, copy and paste); closing the last window removes the Dock icon.

## Final state (output)

- The app runs from its current bundle name with its data in the current folders.

## Check on Linux

- [ ] Known difference, by design: none of this exists on Linux (no app menu bar, no rename history). Nothing to check beyond confirming it is documented.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
