# Vibe Buddy App

The app is Vibe Buddy's graphical interface on the Mac: a menu bar icon plus a settings window. It handles onboarding, settings, the announcement voice and firmware, and it supervises the daemon. It is not a second channel for alerts: agent events are all told by the buddy on the box, and the app speaks up only when the box can't (when the link breaks). This document records the design as settled on 2026-09-16; the three hard-to-reverse trade-offs are covered separately in ADR-0003, 0004 and 0005.

## Role and lifecycle

- **Lives in the menu bar; a Dock icon only while a window is open.** `LSUIElement` is true. Opening the settings or onboarding window switches the activation policy to `.regular`, so the window gets a Dock icon and a menu bar (with ⌘Q still saying the box goes offline); closing the last one switches back to `.accessory`. A permanent Dock icon was rejected because its Quit would take the box offline from a place that doesn't say so.
- **Opening the app again shows Settings.** Double-clicking the running app (Finder, Launchpad, Spotlight) arrives as a reopen event and brings up Settings, or onboarding if it is still open. Before 2026-10-05 it did nothing, and first-time users thought the app hadn't started.
- **One copy at a time.** A second copy with the same bundle id (another path such as a mounted DMG, or a dev build) would start a second daemon that fights over port 7331 and the serial port. On launch the app looks for a running copy; if it finds one, it asks it to open Settings (a distributed notification) and quits before starting anything.
- **Supervises the daemon.** `vibebuddyd` is bundled inside the app as a helper. The app launches it on startup and restarts it after a crash with 1, 2 and 5 second backoff; after three consecutive failures it stops trying, posts a notification, and the menu's first line becomes "daemon not responding · click to restart".
- **Quitting takes the box offline.** Quitting the app quits the daemon, and the menu item says so: "Quit Vibe Buddy (the box goes offline)". The daemon no longer has a lifecycle independent of the app.
- **Replaces the LaunchAgent.** If on first launch the app finds the old `com.vibebuddy.vibebuddyd` LaunchAgent, or port 7331 is already taken, it offers to remove the old one and take over.
- **Launch at login** is registered with SMAppService. Onboarding asks about it in its last step, and it can be changed later on the General tab. The system will say "background item added"; this is the only system permission interaction the user will run into.

## Tech stack and repo layout

- SwiftUI interface, with AppKit's `NSStatusItem` plus a standard `NSMenu` for the menu bar; the daemon and hook stay in Rust.
- The `app/` directory holds the Xcode project, with the `.xcodeproj` checked in directly rather than generated. A Build Phase runs `cargo build --release -p vibebuddyd -p vibebuddy-hook` and copies both binaries into the app bundle.
- Minimum macOS 14. The interface is bilingual (English and Chinese) and by default follows the system language: if the system's preferred language is Simplified or Traditional Chinese it shows Chinese (Traditional Chinese systems temporarily get the Simplified translation; packaging copies `zh-Hans.lproj` to `zh-Hant.lproj`), and everything else gets English.
- The General tab's Language picker (System / English / 简体中文) overrides this by writing the app's own `AppleLanguages` default, which macOS reads at launch, so it asks to restart the app. If the box is online and its voice speaks the other language, the same dialog offers (checked by default) to switch the voice to the first bundled voice in the new language. UI language and voice stay separate settings: the dialog asks rather than switching, because an English UI with a Chinese voice is a reasonable choice and a voice write takes minutes over the UART bridge. With Restart now, the voice is written by the restarted app once the box reports its build; with Later, it is written right away.
- UI strings are written in the Swift sources with English as the key (SwiftUI literals and `String(localized:)`), and the Chinese translations live in `app/Localization/zh-Hans.lproj/Localizable.strings`. English needs no table; `en.lproj` only declares that the app supports English. Packaging copies both lproj folders into `Contents/Resources`, and the development language in `Info.plist` is `en`.
- `tools/check-localization.py` extracts every key from the sources and reconciles them against the Chinese table: a missing translation, an extra entry, or a translation whose placeholder types or argument order don't match the key all make `build-app.sh` fail (with a type mismatch the translation can't be found at runtime and English silently shows instead). Run it after adding or changing any UI string. Int interpolations are written as `%lld` in the table, everything else as `%@`, and translations with several placeholders use positional forms like `%1$@`.
- Messages the daemon sends back to the app (operation progress, failure reasons, HTTP rejection reasons) are all in English. The progress line on the settings page is phrased by the app itself in the interface language, based on the operation type and state; the daemon's original text is attached only on failure (and the hover tooltip likewise appears only on failure). Daemon and firmware logs and errors are in English too.
- Display name `Vibe Buddy`, bundle name `Vibe Buddy.app`, bundle id `com.vibebuddy.app`, helper named `vibebuddyd`. This follows the convention of "change display names, not internal names".
- The daemon reads the signed update manifest (ADR-0010) and the app shows what it finds: a newer App on the General tab, newer firmware on the Device tab. On the Mac, Sparkle installs a newer App from an EdDSA-signed appcast, behind its own window (install, remind me later, skip; no skip below `min_supported_app`). It runs only when the build has the appcast address and `SUPublicEDKey`, and its schedule follows the same switch as the daemon's checks.

## How the app talks to the daemon

It keeps using local HTTP on `127.0.0.1:7331`. New additions:

- `GET /v1/status`: link, current mode, firmware and daemon build identifiers, today's stats, and the time of the most recent hook event.
- `GET /v1/status/stream`: SSE, pushed whenever the status changes.
- Write operations: write a voice pack, update firmware, take a screenshot, restart the daemon itself.

Config is owned by the daemon and stored in `~/Library/Application Support/VibeBuddy/config.json` (current voice, notification toggle and so on). The app reads and writes it through the API and never touches the file directly; the existing environment variables remain as a development-time override. The app itself uses UserDefaults only for interface state such as window position.

## Menu bar

The icon is the buddy's pixel face: normal while the device is connected, grayed out with eyes closed when the link is down or the daemon isn't up. Clicking it opens a standard menu:

1. Device status: `Box online · firmware abc1234` / `Box not found` / `daemon not responding · click to restart`
2. Current mode: On duty / Pomodoro / Leisure
3. A read-only line with today's stats
4. Settings…
5. Check for Updates…: the same as "Check now" in Settings; Sparkle shows its own window for the App
6. Report a Problem…: opens a new GitHub issue in the browser, its body already holding the versions from `summary.txt` (App, daemon, firmware, voice, OS) and nothing else, no logs
7. Quit Vibe Buddy (the box goes offline)

Task cards don't go in the menu; the box is where you look at tasks.

## Notifications

macOS notifications appear only when the link breaks: the device has been disconnected for more than 30 seconds (a quick unplug doesn't trigger one), or the daemon has failed to restart three times. Agent events never produce a notification, to avoid duplicating the box. Notification permission is requested the first time a notification actually needs to be shown, not during onboarding.

## Settings window

Five tabs, styled like System Settings: tabs across the top, grouped forms, system font and colors. Branding appears only in onboarding, the About page and the menu bar icon.

**General**: Launch at login, the toggle for link-failure notifications, and Updates: a "Check for updates" switch (on by default in builds CI makes, off when built from source), "Check now", the outcome of the last check and a download link when a newer App is out. Next to the versions, "Report a Problem…" does what the menu item does. Below the manifest's `min_supported_app`, a banner at the top says the App is no longer supported; it can't be dismissed and nothing stops working.

**Sound**: At the top is a volume slider (20 to 100, in steps of 5). Its value comes from the box's status, and it's written to the box's NVS only on release, so it survives restarts. The box's own menu can change the volume too; the slider follows the `VOLUME` line it prints. Next to it, "Play a line on the box" has the box play "All done!" at the new volume; previews on the Mac are unrelated to it. There's no mute, and the volume can't go to zero; muting happens only on the box, by long-pressing K2. Below that is a list of announcement voice cards. Each card has a name, an accent or source tag, a one-sentence summary of who the character is (so the user can choose between them), a play button (plays all five lines back to back, about ten seconds; click again to stop), a "Use" button and an "In use" badge. After you click "Use", a progress bar appears on the card and the other cards and the Device tab's actions are disabled; over the bridge this takes up to about three minutes, and the card is marked "In use" only after the write finishes and the box reports its verification result. If the cable is pulled partway through, the firmware sees the partition as empty and automatically falls back to the built-in voice; the app prompts you to write it again. No resumable writes.

**Agents**: Two rows, Codex and Claude Code, each showing its status and the time of its most recent event, with "Connect", "Repair" and "Remove". GitHub CLI doesn't appear in the interface in this version; CI polling keeps using the already signed-in `gh`, and if you're not signed in there are simply no CI cards, silently. What gets written is the user-level `~/.claude/settings.json` and `~/.codex/hooks.json`, merging only Vibe Buddy's own entries and leaving other hooks alone, with a diff shown for confirmation before writing. Because it's user-level config, one install covers both places: sessions in the desktop app, and the same agent running in a terminal. The interface has to say this, or users will think the terminal needs a separate install. After writing, the row shows "Waiting for the first event…" and turns green as soon as one arrives. Trusting Codex's `/hooks` can only be done by a person, so the app just prompts for it, but the prompt must be able to tell good from bad: Codex silently disables a changed hook until it's re-trusted, so when the config file is newer than the most recent event the daemon reports, the row turns red and says plainly to go type `/hooks` in Codex. When the app itself rewrites Codex's config (including a rename migration), it posts a notification right away. The app doesn't recompute Codex's trust hash; it only checks whether any event has arrived since the write. Project-level config is never touched.

**Device**: Connection status and serial port name. With several devices plugged in and none of them the box seen before, it lists them (port and USB serial number) with "This is the box"; the daemon remembers the pick as it would a box that reported our firmware (`POST /v1/device/choice`). When `VIBEBUDDY_SERIAL_PORT` or `VIBEBUDDY_USB_SERIAL` narrows the search, it says so and how to unset it, in place of the usual "not showing up" help: a pin left set once made a swapped box look unfound. Then "Box firmware" and "Latest firmware", with "Update to x" when a newer firmware is downloaded; the box's current screen, captured only when you click "Refresh" (over the bridge one capture ties up the serial port for a few seconds, so there's no automatic polling), with "Save image".

**Advanced**: Open logs, restart the daemon, export diagnostics (logs, config and both sides' build identifiers bundled into one package).

## Onboarding

1. Welcome.
2. Find the box: prompts you to plug in the box, lists the ports it finds, and once connected has the box blink to confirm. Can be skipped, after which the menu bar keeps showing "Box not found".
3. Connect agents: detects Codex and Claude Code, writes the hooks in one click, and prompts you to trust them in Codex's `/hooks`. You can choose "Not now", but you can't skip this page.
4. Pick a voice: preview and write it to the box. Can be skipped.
5. Launch at login.
6. Done.

Finding the box comes before connecting agents: seeing the box come alive first, before configuring anything else, builds confidence.

## Announcement voices and voice packs

The firmware no longer compiles the five PCM lines into the program. The partition table gains a 2 MB `voices` data partition that the firmware reads at boot; if the partition is empty or fails verification, the factory built-in voice is used (currently Jessica, in English).

A voice pack is a small custom format with no file system: magic number, version, voice id, five `[offset, length]` entries, followed by five PCM segments in the same format as the existing assets. The header is written last, so a half-written pack counts as not written.

Writing goes over the existing serial protocol: the Mac sends base64 in chunks, and the firmware writes the partition itself, verifies it and reports back, without resetting or grabbing the serial port; both connection types behave the same. Over the bridge at 115200 baud, 1.5 MB takes about three minutes; the native USB port takes a few seconds. The esptool path is reserved for firmware updates.

The packs the app ships are the Character packs committed as `characters/*/pack.bin` (see [`characters.md`](characters.md)), about 2.5 MB for four.

The voice catalog `VoiceCatalogEntry.all` tags each voice with the language of its lines (`VoiceLanguage.zh` / `.en`). The voice picker lists only the voices this build actually ships a pack for (computed once at startup from whether the file exists), and puts voices matching the interface language first. The catalog has five English Characters, `ada`, `hank`, `luna`, `kai` and `mei`, and one Chinese, `wanwanxiaohe`. The firmware's built-in voice is Jessica, in English, so onboarding in the Chinese interface suggests picking a Chinese voice.

New events the device protocol needs (a draft, to be written into `protocol.md` at implementation time): `device.identify` (blink), `voice.begin` / `voice.chunk` / `voice.end`, and the device reporting `voice.written` with the verification result.

## Firmware updates

Firmware isn't bundled with the app; it is released on its own (`firmware-vX.Y.Z`) and listed in the update manifest with the oldest App each version runs with (ADR-0010). The daemon picks the newest firmware this App can run, downloads it, checks its sha256 and keeps it unpacked in its state directory, so later updates and recovery work offline. The app only shows what the daemon offers.

"Update to x" appears only when the box runs something older, a build without a version (firmware from before ADR-0010) or a `-dirty` build of the same version, and only once the download is on disk; a newer box is never downgraded. Nor is it offered to our firmware built for other hardware: a build that prints `BOARD x` for anything but `alientek-box` (the breadboard devkit says `goouuu-s3-spi`) gets a note instead, since released firmware is built for the box alone and starts neither screen, speaker nor buttons elsewhere. Released firmware prints no `BOARD` line, so the box itself is unaffected. One notification per new firmware version is posted while the box is connected (macOS only for now). When firmware is wanted but not on disk (update checks off, a failed download, or still downloading), the Device tab and onboarding say why and point at the releases page and "Flash from file…".

Flow: confirm → daemon releases the serial port → espflash flashes (automatically no-stub over the bridge) → box restarts → reconnect, with progress visible throughout. On failure it offers a retry and a fallback instruction to "hold K0 and replug the cable". Updates don't erase the `voices` and `nvs` partitions, so changing firmware keeps the voice and today's stats. Never updates automatically.

When the Mac can't see the box at all, onboarding's "Find the box" page and the Device tab give two steps in order. First the everyday causes: a charge-only cable, or not clicking Allow when macOS asks whether to let the accessory connect. Only then K0: holding it while plugging in starts the ROM bootloader instead of the firmware (K0 is the BOOT button on GPIO0), so the box shows up with a dark screen and no build number and the page offers to flash it. K0 is never suggested as a general fix, because a box in download mode does nothing until it is flashed.

The firmware zip, `VibeBuddy-firmware-vX.Y.Z.zip` (the trio plus `build.txt`, `version.txt` and license notices), is attached to each firmware release. Choosing it via "Flash from file…" on the Device tab unzips it to a temporary directory, checks the magic numbers of the three images, reads out the build identifier and version, shows "Firmware package" and "Box now" side by side in a confirmation dialog, and then goes down the same flashing path. This is the way in when the daemon can't download firmware.

A factory-fresh box (running some other firmware) goes down the same flashing path with the downloaded firmware, just from a different entry point, so onboarding needs the network once. Every time the daemon opens the port it resets the build number and sends hello; if no build number comes back, it concludes the box isn't running our firmware (how it tells, and why it then writes the box nothing, is decision 17 in [`architecture.md`](architecture.md)) and reports `foreign_firmware` in the status: the box is there, not offline. Both the onboarding "Find the box" page and the Device tab switch to offering "Flash Vibe Buddy firmware", with a confirmation dialog that says plainly the existing firmware and data will be erased. The ROM download protocol doesn't care what the box was running before; but the native USB port is only recognized if the other firmware kept USB Serial/JTAG (VID/PID `303A:1001`). If it isn't recognized, the user is told to switch to the UART port, which is a hardware bridge and independent of the firmware.

## Build identifier on the box

The box shows the Mac side's build on the `APP` row of the device menu's STATUS view (see [`pet.md`](pet.md#build-identifiers)); the heartbeat's `build` is the app version plus the build number. It isn't cleared when the link drops; it goes gray, because when disconnected the most valuable information is precisely "which version was connected last".

## Implementation notes (2026-09-16)

- The app for daily use is installed at `/Applications/Vibe Buddy.app`, delivered and launched with `app/scripts/build-app.sh --install`. The login item and hook paths are both tied to the bundle's location; `app/build` in a worktree is just a development artifact.
- The build uses SwiftPM plus a packaging script, not an Xcode project: this machine has only the Command Line Tools, so `xcodebuild` isn't available, while `swift build` can compile SwiftUI and AppKit. With Xcode installed you can open `app/Package.swift` directly. The Command Line Tools also lack XCTest and swift-testing, so the view models' seams are guarded by assertion self-checks in `swift run --package-path app SelfTest`.
- Firmware flashing doesn't use espflash: its ROM write block is fixed at 1 KB and its serial port object is a concrete type that can't be wrapped, so it can't get through the UART bridge. The daemon implements the ROM download protocol itself, with 256-byte blocks, segmented at line speed over the bridge, and a hard reset after MD5 verification.
- Chunked voice pack writes run at about 5 KB/s over the bridge (1.2 MB in four minutes), somewhat slower than the design estimated; the bottleneck is stop-and-wait acknowledgments plus 115200 baud. The native USB port is much faster.
- Once the device connects, the daemon first gets `device.hello`, which re-reports mode, firmware build number and voice; otherwise the daemon would know nothing after restarting.
- The app passes its own pid to the daemon in `VIBEBUDDY_PARENT_PID`; if the app is force-killed (SIGKILL), the daemon exits on its own within two seconds instead of becoming an orphan that holds the serial port and the network port. SIGTERM is caught by the app, which goes through a normal quit.
- The packaging script does ad-hoc signing only so the login item and notifications can recognize the bundle's identity; it isn't Developer ID signing and notarization for distribution, which are still under "Explicitly out of scope".
- The offsets of the firmware trio (0x0, 0x8000, 0x10000) are determined by the partition layout and hard-coded in the daemon; the app only passes file paths.

## Explicitly out of scope

- **Mute**: only by long-pressing K2 on the device; the Mac doesn't record it, show it or toggle it.
- Task cards in the menu, and macOS notifications for agent events.
- Installing any update without the user's confirmation, quiet hours, project-level hook config.

## Acceptance

Walk through on real hardware: all six onboarding steps; change the voice once and hear the new voice; update the firmware once with the voice and stats still intact; get a notification 30 seconds after unplugging, and see the icon recover after plugging back in; after writing the hooks in one click, the Agents tab turns green on the next agent event.
