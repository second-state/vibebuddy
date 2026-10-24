# AGENT-02: Connect Claude Code or Codex (entries merged into the user's config)

Category: Agents tab · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/SettingsView.swift` (`HookRow`, `PlanSheet`)
- `app/Sources/VibeBuddy/HookInstaller.swift` (`installPlan`, `apply`)
- `app/Sources/VibeBuddyCore/HookConfig.swift`

## Initial state (input)

- The agent is present and not connected.

## Trigger

"Connect".

## Workflow (Mac app)

1. The app computes a plan for `~/.claude/settings.json` or `~/.codex/hooks.json`: VibeBuddy's entries are removed, then one entry per event is added (`"<binary>" claude|codex`, timeout 2). Claude Code gets 8 events; Codex 6. Nobody else's hooks are touched.
2. A sheet shows "Will change {path}" and the diff, one line per event (`+ Event: command` in green, `- …` in red), or "Nothing to change." with Write disabled.
3. For Codex the sheet adds "After writing, open /hooks in Codex to review and trust this config — the app can't do that for you."
4. Write: the hook binary is copied into place again, a `.bak` copy of the existing file is kept, and the new file is written atomically. Cancel writes nothing.
5. On failure: "Couldn't write the hook config: {reason}".

## Final state (output)

- The row shows connected (AGENT-05) with Repair and Remove.

## Check on Linux

- [ ] Does Linux show the exact change before writing, keep a backup, touch only VibeBuddy's entries, and add the Codex trust note? (Linux plans through `vibebuddy-hook plan`.)

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
