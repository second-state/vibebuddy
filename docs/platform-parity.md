# Computer side: Mac and Linux parity review

What the computer side does on each platform, what differs, and why. The device side is out of scope. This review reads
the code (`app/`, `desktop/`, `daemon/`, `hook/`, `packaging/`, the CI workflows); each row names the code it rests on.
Where a behaviour depends on a third-party app or desktop environment and nobody has run it, the row says unverified.

Reviewed 2026-10-17, second pass (the first pass had errors; see "Corrections" at the end).

## How the two apps are built

| | Mac app (`app/`) | Linux app (`desktop/`) |
|---|---|---|
| Toolkit | SwiftUI and AppKit, menu bar | Rust, iced window and ksni tray |
| Daemon supervision | the app starts and restarts `vibebuddyd` (`DaemonSupervisor.swift`, backoff `[1,2,5]` s, gives up after three failed starts) | systemd user service (`packaging/linux/vibebuddyd.service`: `Restart=always`, `RestartSec=2`, default start limit) |
| Quitting the app | stops the daemon: the box goes offline ("Quit VibeBuddy (the box goes offline)") | leaves the daemon running: the box stays online (`tray.rs`, "Quit") |
| Install | Homebrew cask, DMG | `install.sh` from the release tarball, pacman package, or build from source |
| Tests in CI | `SelfTest` only (model and status decoding), macOS runner | `cargo test --workspace` on Ubuntu (daemon, hook, Linux app), clippy, the localization check, the Python tool tests, and the Linux host suite with a simulated box (`tools/test-linux-host.sh`, see `docs/linux-host-testing.md`) |

The daemon (`daemon/`) and the hook (`hook/`) are shared. Their platform code sits in a few places: the config and
state directories (`daemon/src/config.rs`), K2 (`daemon/src/source_opener.rs`), the box-lost alert
(`daemon/src/link_alert.rs`, started only outside macOS, `daemon/src/main.rs`), and where the hook says the agent runs
(`hook/src/surface.rs`).

## Feature matrix

Status key: **same**; **differs** (both do it, by a different mechanism or with a different result); **Mac only**;
**Linux only**; **by design** (a difference that follows from the platform and is documented as intended).

### Settings window

| Area | Mac | Linux | Status |
|---|---|---|---|
| Tabs: General, Character, Agents, Device, Advanced | yes | yes | same |
| Language override: System, English, Chinese | `AppLanguage.swift` | `i18n.rs` (`UiLanguage::ALL`) | same |
| Launch at login | `SMAppService` (`LoginItem.swift`) | the toggle writes or hides the user's XDG autostart entry; the pacman package ships a system-wide one (`main.rs`, `set_launch_at_login`) | differs, by design |
| Relay toggle; check for updates toggle; Check now; Report a Problem | yes | yes (Report a Problem opens the same issue URL with the diagnostics summary) | same |
| App updates | Sparkle installs them from the app | the app says a newer version exists and how to upgrade (rerun `install.sh`); the Releases button opens the page | differs, by design |
| Character: built-in, your own (choose images, copy a prompt), voice and lines, preview | yes; previews play on the Mac (`VoicePreview.swift`) | yes; previews play through `pw-play`, `paplay` or `aplay` (`preview.rs`) | same (different player) |
| Volume, "Play a line on the box" | yes | yes | same |
| Agents: connect, repair, remove, the exact change shown first; Codex `/hooks` trust hint | yes | yes (`agents.rs`, `CodexTrust`) | same |
| Agents: terminal tabs and the Automation permission | yes (`TerminalAccess.swift`: Ghostty, iTerm2, Terminal) | no | Mac only |
| Device: Wi-Fi (scan, join, forget, change), paired computers, take over, make the box blink, box screen (refresh, save) | yes | yes | same |
| Device: firmware update, flash from a firmware zip, retry with the K0 hint | yes | yes (`firmware_file.rs`) | same |
| Device: several boards plugged in ("This is the box"), the port pin notice, cable help | yes; the help also mentions macOS's accessory permission | yes; the help also points to "Permission denied" in the daemon log (the serial group) | same, each with its platform's hint |
| Advanced: daemon status, Open logs folder, Restart daemon, Export diagnostics, config path | yes; restart works while down | yes, plus Show daemon log (in `xdg-terminal-exec`, else GNOME Terminal, Konsole, Xfce Terminal or xterm); restart uses systemd while up or down, as the tray does, and reports errors | Linux has one more |
| First-run guide | includes connecting agents and launch at login | Welcome, Find the box, Pick a character, Wi-Fi, Done (`install.sh` already connected agents and set autostart; `docs/app.md`) | differs, by design |

### Notifications

| Notification | Mac | Linux | Status |
|---|---|---|---|
| Box gone for 30 s | the app (`AppModel.swift`, `checkLink`) | the daemon, through `notify-send` (`link_alert.rs`) | same rule, different sender |
| New box firmware available | the app (`AppModel.swift`) | the app, through `notify-send` (`main.rs`, `notify_new_firmware`) | same |
| Daemon failed | the app, when its supervisor gives up (`AppModel.swift`, `onGiveUp`); retains the last reported notify toggle | the app, through `notify-send` (`outage.rs`), once after 30 s without the daemon and again only after recovery. An inactive/deactivating unit suppresses the alert and is rechecked after another 30 s, so a later failed restart can notify | same last-known-toggle rule (on before the first status); different supervisors and failure timing |
| "Codex hook config updated" | the app, after migrating legacy hook commands (`HookInstaller.migrateLegacyCommands`) | no such migration on Linux | Mac only, by design |

### Menu bar and tray

| Area | Mac | Linux | Status |
|---|---|---|---|
| Icon | `NSStatusItem` with the buddy's face | ksni StatusNotifierItem tray icon with the face | differs (platform) |
| Menu: device, mode and today lines; restart from the device line when the daemon is down; Settings…, Check for Updates…, Report a Problem…, Quit | yes | yes (`tray.rs`, `menu`) | same |
| App menus (Edit, Window, Hide) | standard macOS menus | none (no app menu bar) | by design |
| No tray host | not applicable | the app notices (`watcher_offline`, or ksni fails to start) and opens the settings window after 10 s (`main.rs`, `TrayHostCheck`); the README's Linux section says why and names the extension | differs; explained in the README, not in the window |
| Omarchy bar plugin, Omarchy colours and font | not applicable | yes (`tray.rs`, `theme.rs`) | Linux only |

Where a tray host exists: KDE supports StatusNotifierItem natively; Ubuntu preinstalls the AppIndicator GNOME extension;
Fedora and upstream GNOME do not, so there the icon needs the extension. Sources in the research notes. Without a
tray host the app opens its settings window about 10 s after start (`TrayHostCheck`); the README's Linux section says
why and names the extension.

### K2: what the box's source key opens

K2 goes back to the agent. The hook reports where the agent runs, and the daemon acts on that
(`hook/src/surface.rs`, `hook/src/claude.rs`, `daemon/src/source_opener.rs`).

On macOS the hook reports the host app's bundle ID (`__CFBundleIdentifier`), the tty or tmux pane, and, for a Claude
desktop app session, its session ID. Outside macOS the hook always reports the agent's process ancestors and any tmux
pane (`Surface::Window`); it never reports a desktop session ID (`desktop_session_id` is only added for `Surface::App`).

| Where the agent runs | Mac | Linux | Status |
|---|---|---|---|
| Claude Code session in the Claude desktop app | opens that session: `claude://code/continue?session=…` | focuses the window that owns the agent's process, on Hyprland, Sway or X11; that is the app window, not the session | Linux gap: window, not session; unverified on the Claude Linux beta |
| Codex in the Codex app | opens that thread: `codex://threads/<id>` | same as above: the app window on Hyprland, Sway or X11 | Linux gap; unverified on the ChatGPT/Codex Linux preview |
| Terminal session | brings the terminal forward; with the Automation permission, the exact tab (Ghostty, iTerm2, Terminal) | focuses the terminal window: `hyprctl` on Hyprland, `swaymsg` on Sway, `wmctrl` on X11 (`window_focus.rs`); the window system is read from systemd's user manager at each press, then from the daemon's own environment | differs; tabs Mac only |
| Warp | its focus deep link back to the pane | not handled (the hook ignores `WARP_FOCUS_URL` outside macOS) | Mac only |
| Inside tmux | switches the tmux client to the pane, then the terminal tab | switches the tmux client to the pane (any desktop), then focuses its window (Hyprland, Sway or X11) | differs |
| GNOME or KDE under Wayland (no supported window backend) | not applicable | no window focus; K2 logs the failure; a tmux pane switch still happens | **Linux gap** |
| GitHub Actions card | `open` the run page | `xdg-open` the run page | same |
| No host (SSH, background process) | nothing to open, logged | same | same |

### Platform detection, paths, packaging

| Area | Mac | Linux | Status |
|---|---|---|---|
| Finding the box | by USB vendor and product ID (Espressif native USB, QinHeng CH343), preferring `/dev/cu.*` | the same ID match, any port name (`/dev/ttyACM*`) | same |
| Port access | macOS may ask to allow the accessory | the user must be in the group that owns the port (`uucp` or `dialout`); `install.sh` says so, and the app's help points to the daemon log | differs, by design |
| Config and state | `~/Library/Application Support/VibeBuddy` | XDG: `~/.config/vibebuddy`, `~/.local/state/vibebuddy` | differs, by design |
| Release artefact | DMG, Homebrew cask; Developer ID and notarisation when the secrets are set; Sparkle's EdDSA appcast | tarball with `install.sh`, pacman package | differs, by design |
| Update manifest | Ed25519-signed, key `macos-arm64` | the same manifest, key `linux-x86_64`, with the tarball's SHA-256; nothing checks it on a manual download | same manifest |

## Gaps, ranked

1. **K2 focus: Hyprland, Sway and X11 now; GNOME and KDE under Wayland still not.** On GNOME and KDE Wayland, K2 does
   nothing visible except a tmux pane switch. The Sway and X11 backends (`daemon/src/window_focus.rs`) are checked
   headless against synthetic window listings; desktop acceptance on each is pending. Before this change, the gap
   covered every desktop but Hyprland.
2. **Daemon-failure notification: implemented; desktop acceptance pending.** The Linux app sends it (`outage.rs`)
   and rechecks intentional-stop suppression. Both apps retain the last reported toggle after daemon loss. Linux
   bounds status response headers to 10 s and stream inactivity to 45 s before reporting the daemon down; the Mac
   uses a 45 s stream inactivity timeout. SSE keepalives keep quiet healthy streams alive. Advanced and tray restart
   both use systemd on Linux, including while the daemon is down. Real notification/UI acceptance remains pending.
3. **K2 reaches the Claude or Codex app window, not the session.** Focus works on Hyprland, Sway and X11. Whether
   the Linux builds of those apps set `CLAUDE_CODE_HOST_SESSION_ID` and register the `claude:` and `codex:` URL schemes is
   unknown: the apps are not installed on the machine where this was checked, so this has not been checked. The Claude
   Code CLI's desktop entry registers `claude-cli://`, not `claude://`.
4. **No explanation when there is no tray: documented, not in the window.** The window opens after 10 s, and the README's
   Linux section now says why and how to get the icon. A line in the window is deferred until users report
   missing it. Checked on Ubuntu 22.04 GNOME (extension on, host present: no window, tray item registered) and in an
   isolated Xvfb run without a watcher (window appears 9–11 s after start). Fedora and upstream GNOME are not run.

By design, and documented rather than closed: manual app updates on Linux, the opposite meaning of Quit, the terminal
tabs and Warp (Mac only), the app menus, the first-run guide's shorter Linux path.

## Verified and unverified

Verified by reading the code: every row above.

Checked headless, with no desktop: the daemon-failure notification policy and its probe (`DaemonOutageTests`), the Sway and
X11 focus backends and their selection (`SwayFocusTests`, `X11FocusTests`, `daemon/src/window_focus.rs`), the Hyprland K2
regressions, and the tray host's behaviour in isolation (an Xvfb display with a private bus, and with the user's bus).

Not run on a machine: K2 on a real Sway or X11 session; the daemon-failure notification on a real desktop and systemd
unit; the tray without the AppIndicator extension on Fedora, upstream GNOME or KDE; the notifications under each desktop
environment; the Linux voice preview; the Linux Claude and Codex apps' environment and URL schemes (that needs the apps
installed); the Mac app's give-up path and its retained toggle and stream timeout (read from the code only;
not compiled on Linux).

## Research notes (web, 2026-10-17; secondary sources, check the vendors before relying on them)

- Sway: `GET_TREE` returns the node tree with a pid per window; `swaymsg` focuses a node by criteria
  ([sway-ipc(7)](https://man.archlinux.org/man/sway-ipc.7)).
- X11: `wmctrl -l -p` lists windows with their pid; `wmctrl -i -a` activates one; `xdotool search --pid` is the
  alternative. Pid matching needs the client to set `_NET_WM_PID`
  ([wmctrl(1)](https://man.archlinux.org/man/wmctrl.1.en.raw), [xdotool(1)](https://manpages.debian.org/wheezy/xdotool/xdotool.1.en.html)).
- GNOME: StatusNotifierItem needs the AppIndicator extension
  ([gnome-shell-extension-appindicator](https://github.com/Finii/gnome-shell-extension-appindicator)); Ubuntu preinstalls
  it, Fedora packages it separately, KDE supports the protocol natively
  ([Fedora desktop list](https://lists.fedorahosted.org/archives/list/desktop@lists.fedoraproject.org/2019/11/)).
- systemd: a user service can notify on failure with `OnFailure=`
  ([makandracards](https://makandracards.com/operations/529660-desktop-notification-failed-systemd-user-services?read=true));
  with `Restart=always` it fires only once the start limit is hit ([freedesktop bug 87799](https://bugs.freedesktop.org/show_bug.cgi?id=87799)).
- Claude desktop for Linux: a beta for Ubuntu 22.04+ and Debian 12+, June 2026
  ([report](https://pasqualepillitteri.it/en/news/6967/claude-desktop-linux-beta-ubuntu-debian-en)).
- ChatGPT desktop for Linux with Codex: a preview, August 2026, for Ubuntu, Debian and Fedora
  ([report](https://aiweekly.co/node/9842)).

## Corrections to the first pass

- K2 into the Claude and Codex apps: the first pass said Linux builds the deep link and fails on macOS's `open`. The
  Linux hook never reports the session, so the daemon does window focus instead; it never reaches `open` there.
- Notifications: the first pass missed the Linux firmware-available notification and the Mac-only Codex-config notice.
- Tray: the first pass said the Linux app says nothing without a tray; it detects this and opens the window. It also
  understated the Linux tray menu, which matches the Mac's.
- Launch at login: the Linux toggle writes the autostart entry itself, not only `install.sh`.
- Signing: the Linux tarball's checksum is in the signed manifest; the first pass said nothing was documented.
- Removed: the claim about `/dev/ttyUSB*` (not checked), and "CI covers less of the Mac app" as a Linux gap (it is not
  one).
