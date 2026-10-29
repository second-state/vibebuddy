# Mac app scenarios for Linux parity checks

Each note describes one thing the Mac app (`app/`) does, as read from its code: the state before, what sets it
off, the steps the app takes, and the state after. They are the reference for checking that the Linux app
(`desktop/`) behaves the same, as `AGENTS.md` asks ("Mac and Linux apps stay in step").

The notes describe the Mac app only. Where `docs/platform-parity.md` already records a difference as by design,
the note's "Check on Linux" list says so; everything else there is a question to answer, not a claim about Linux.
UI strings are quoted as the English keys in the Swift source; the zh-Hans text is in
`app/Localization/zh-Hans.lproj`. A word in braces, such as {version}, stands for a value filled in at run time.

## How to use a note

1. Set up the initial state on Linux (the host suite, `tools/test-linux-host.sh`, covers many daemon-side states).
2. Do the trigger and compare each workflow step and the final state with the Linux app.
3. Tick the "Check on Linux" items, and fill in "Linux result" with one status: `same`, `differs by design`
   (documented in `platform-parity.md`), `gap` (open an issue, or fix it in both apps), or `not applicable`.
4. When a gap is closed or a difference is accepted, update `platform-parity.md` too.

When the Mac app changes, update the note in the same change.

## Scenarios

### App lifecycle and daemon supervision

| Scenario | Title |
|---|---|
| [LIFE-01](LIFE-01-first-launch.md) | First launch: the daemon starts and the first-run guide opens |
| [LIFE-02](LIFE-02-later-launch.md) | Later launches: menu bar only, or Settings after a relaunch |
| [LIFE-03](LIFE-03-second-copy-or-reopen.md) | Opening the app again while it runs brings its window forward |
| [LIFE-04](LIFE-04-quit.md) | Quit: the app stops its daemon and the box goes offline |
| [LIFE-05](LIFE-05-daemon-crash-restart.md) | The daemon crashes and the app restarts it with backoff |
| [LIFE-06](LIFE-06-daemon-gives-up.md) | The daemon fails three times in a row and the app gives up |
| [LIFE-07](LIFE-07-restart-daemon.md) | Restart the daemon on request |
| [LIFE-08](LIFE-08-old-daemon-found.md) | An old daemon is found at launch |
| [LIFE-09](LIFE-09-status-stream-liveness.md) | The status stream decides whether the daemon is alive |
| [LIFE-10](LIFE-10-mac-only-housekeeping.md) | Mac-only housekeeping: bundle rename, legacy folders, Dock icon and app menus |

### Menu bar

| Scenario | Title |
|---|---|
| [MENU-01](MENU-01-menu-daemon-down.md) | Menu bar while the daemon is down |
| [MENU-02](MENU-02-menu-box-online.md) | Menu bar while the box is online |
| [MENU-03](MENU-03-menu-box-not-found.md) | Menu bar while the daemon runs but no box is linked |
| [MENU-04](MENU-04-menu-actions.md) | Menu bar actions: Settings, Check for Updates, Report a Problem, Quit |

### Notifications

| Scenario | Title |
|---|---|
| [NOTIF-01](NOTIF-01-box-gone-30s.md) | Notify once when the box has been gone for 30 seconds |
| [NOTIF-02](NOTIF-02-firmware-available.md) | Notify once per new firmware version |
| [NOTIF-03](NOTIF-03-codex-hooks-rewritten.md) | Tell the user when the app rewrote Codex's hook config |

### General tab

| Scenario | Title |
|---|---|
| [GEN-01](GEN-01-language-change-with-voice.md) | Change the UI language, restart, and switch the box's voice too |
| [GEN-02](GEN-02-language-change-box-absent.md) | Change the UI language while the box isn't connected |
| [GEN-03](GEN-03-launch-at-login.md) | Turn Launch at login on or off |
| [GEN-04](GEN-04-notify-toggle.md) | Turn the disconnect and daemon-failure notifications on or off |
| [GEN-05](GEN-05-relay-toggle.md) | Turn the VibeBuddy Relay on or off |
| [GEN-06](GEN-06-update-checks.md) | Turn update checks on or off and read their summary |
| [GEN-07](GEN-07-check-now.md) | Check for updates now |
| [GEN-08](GEN-08-app-update-offer.md) | A newer app version is offered |
| [GEN-09](GEN-09-unsupported-app.md) | This app version is no longer supported |
| [GEN-10](GEN-10-report-a-problem.md) | Report a problem |

### Character tab

| Scenario | Title |
|---|---|
| [CHAR-01](CHAR-01-volume-slider.md) | Set the box's volume |
| [CHAR-02](CHAR-02-play-a-line.md) | Play a line on the box at the chosen volume |
| [CHAR-03](CHAR-03-preview-on-computer.md) | Preview a Character on the computer |
| [CHAR-04](CHAR-04-use-a-character.md) | Write a Character to the box |
| [CHAR-05](CHAR-05-character-write-fails.md) | A Character write fails |
| [CHAR-06](CHAR-06-use-the-robot.md) | Go back to the robot, the default Character |
| [CHAR-07](CHAR-07-form-of-address.md) | Choose what the buddy calls you |
| [CHAR-08](CHAR-08-custom-character.md) | Make and use your own Character |

### Agents tab

| Scenario | Title |
|---|---|
| [AGENT-01](AGENT-01-agent-not-found.md) | An agent isn't installed on this computer |
| [AGENT-02](AGENT-02-connect-merged-config.md) | Connect Claude Code or Codex (entries merged into the user's config) |
| [AGENT-03](AGENT-03-connect-owned-file.md) | Connect OpenCode, GitHub Copilot CLI or Pi (a file of VibeBuddy's own) |
| [AGENT-04](AGENT-04-repair-remove.md) | Repair or remove an agent's connection |
| [AGENT-05](AGENT-05-agent-status-and-codex-trust.md) | Each agent's status dot, and the Codex trust hint |
| [AGENT-06](AGENT-06-hook-binary-upkeep.md) | At every launch the hook binary is refreshed and stale hook commands are fixed |
| [AGENT-07](AGENT-07-terminal-tab-access.md) | Allow K2 to reach a terminal's exact tab (Automation access) |

### Device tab

| Scenario | Title |
|---|---|
| [DEV-01](DEV-01-link-line.md) | Device: what the Link line says |
| [DEV-02](DEV-02-box-with-another-computer.md) | The box is with another computer: take it over |
| [DEV-03](DEV-03-several-devices.md) | Several devices are plugged in: say which is the box |
| [DEV-04](DEV-04-pin-and-not-found-help.md) | The box isn't found: the pin notice and the help |
| [DEV-05](DEV-05-firmware-update.md) | Update the box's firmware over the cable |
| [DEV-06](DEV-06-firmware-needs-cable.md) | Firmware actions need the cable, unless the firmware updates over the link |
| [DEV-07](DEV-07-flash-fails.md) | A firmware flash fails |
| [DEV-08](DEV-08-flash-needs-replug.md) | Firmware written, but the box didn't start it |
| [DEV-09](DEV-09-foreign-firmware.md) | The box runs other firmware: flash VibeBuddy |
| [DEV-10](DEV-10-firmware-unavailable.md) | No firmware to flash yet, and what Latest firmware says |
| [DEV-11](DEV-11-unsupported-board.md) | Our firmware on other hardware: no update offered |
| [DEV-12](DEV-12-flash-from-file.md) | Flash a firmware zip chosen by the user |
| [DEV-13](DEV-13-make-the-box-blink.md) | Make the box blink |
| [DEV-14](DEV-14-wifi-join.md) | Put the box on Wi-Fi |
| [DEV-15](DEV-15-wifi-errors.md) | The box can't join the network |
| [DEV-16](DEV-16-wifi-change-forget.md) | Change or forget the box's network |
| [DEV-17](DEV-17-paired-computers.md) | See and remove the computers paired with the box |
| [DEV-18](DEV-18-box-screen.md) | See and save what the box's screen shows |

### Conversation tab

| Scenario | Title |
|---|---|
| [CONV-01](CONV-01-conversation-tab-shown.md) | The Conversation tab appears only while the daemon runs the resident session |
| [CONV-02](CONV-02-conversation-history.md) | Read and follow the voice commands and the buddy's replies |
| [CONV-03](CONV-03-conversation-agent-and-key.md) | Choose the Agent that answers, and store an Anthropic API key |
| [CONV-04](CONV-04-conversation-not-ready.md) | Voice commands aren't ready: say what's missing |

### Account tab

| Scenario | Title |
|---|---|
| [ACCT-01](ACCT-01-sign-in.md) | Sign in to the VibeBuddy account through the browser |
| [ACCT-02](ACCT-02-sign-out-and-delete.md) | Sign out, or delete the account |
| [ACCT-03](ACCT-03-account-unavailable.md) | The Account tab when the daemon is down or can't sign in |

### Advanced tab

| Scenario | Title |
|---|---|
| [ADV-01](ADV-01-advanced-status-and-config.md) | Advanced: the daemon's status and the config path |
| [ADV-02](ADV-02-open-logs.md) | Open the logs folder |
| [ADV-03](ADV-03-export-diagnostics.md) | Export diagnostics |

### First-run guide

| Scenario | Title |
|---|---|
| [ONB-01](ONB-01-guide-navigation.md) | First-run guide: pages and navigation |
| [ONB-02](ONB-02-guide-find-the-box.md) | First-run guide: Find the box |
| [ONB-03](ONB-03-guide-flash-progress.md) | First-run guide: flashing from Find the box |
| [ONB-04](ONB-04-guide-agents-and-character.md) | First-run guide: Connect agents and Pick a character |
| [ONB-05](ONB-05-guide-wifi.md) | First-run guide: Wi-Fi |
| [ONB-06](ONB-06-guide-login-and-done.md) | First-run guide: Launch at login and Done |

### Errors

| Scenario | Title |
|---|---|
| [ERR-01](ERR-01-rejected-action-alert.md) | An action the daemon rejects shows an alert |

77 scenarios, read from the Mac app's code on branch `linux-parity` after merging upstream `main` at `569f892`.
