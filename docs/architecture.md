# Vibe Buddy Architecture

## Goals and boundaries

Vibe Buddy turns status events from local programs into pictures and sounds on a physical desk pet. The ESP32 handles only deterministic rendering, announcements and device I/O; task aggregation, prioritization and client adaptation live on the Mac.

## Components

### `vibebuddyd`

- Receives generic events on `POST /v1/events`, and minimized agent lifecycle events on `POST /v1/codex-hooks` and `POST /v1/claude-hooks`.
- Validates and encodes Vibe Buddy Protocol messages.
- Manages device discovery, the serial connection, reconnection after a drop, and device state.
- Routes button events reported by the device back to local consumers.
- Aggregates sessions from all agents into at most 3 task cards, "newest on top"; activities that need user input take priority in driving the pet's global state.
- Periodically clears activities that have gone a long time without events, so a session that exited abnormally can't hold a task card forever.

### `beacon`

- A client of `vibebuddyd` that provides a command-line interface.
- Never discovers, opens or claims the physical serial port.
- Implemented in Stage 6; Stage 2 verifies the full link with plain HTTP requests first.

### `vibebuddy-fw`

- Receives and parses Vibe Buddy Protocol messages.
- Drives the LCD and speaker; the buddy's animation runs entirely on the box and doesn't depend on the Mac pushing frames one by one.
- Doesn't interpret any local business semantics beyond the task lifecycle.

### `app` (design settled, not yet implemented)

- A menu bar app that supervises `vibebuddyd` (shipping, starting, restarting and quitting it as a helper), replacing the LaunchAgent.
- Handles onboarding, connecting agents (writing hook config), picking an announcement voice and writing it to the device, updating firmware, and viewing the device's screen.
- Speaks up itself (via macOS notifications) only when the link breaks; agent events are still told by the device. See [`app.md`](app.md) for the design.

### `protocol`

- Defines the Vibe Buddy Protocol's message model, encoding and decoding, versioning and limits.
- Independent of the transport, whether USB UART, USB CDC, WebSocket, TCP or Bluetooth.

## Data flow

```text
Codex Hook -------> privacy filter -> POST /v1/codex-hooks ---+
Claude Code Hook -> privacy filter -> POST /v1/claude-hooks --+
Other producer ------------------> POST /v1/events -----------+-> vibebuddyd -> USB -> ESP32-S3
Codex / Claude / Browser <---------- open source <------------+<- vibebuddyd <- K2
```

The first version implements only `SerialTransport`. The transport interface abstracts just the minimum needed to connect, send, receive and close; no elaborate plugin system gets designed until a second real transport exists.

## Key decisions

1. Phase one uses a monorepo, with Rust on both the Mac and the firmware: the firmware is esp-hal + embassy (no_std), logic that doesn't touch hardware lives in `firmware-rs/core` and is tested directly on the Mac, and both ends share a single copy of the protocol types (ADR-0006). The C firmware stays as a fallback until on-device verification passes.
2. The CLI must go through the daemon, so multiple processes don't compete for the serial port and reconnection logic doesn't get scattered.
3. Vibe Buddy Protocol v1 uses NDJSON: one JSON object per line.
4. Firmware hardware configuration must come from official documentation for the exact board model or from on-device verification; never borrow GPIO assignments from a similar board.
5. Compiling, a simulated link and on-device verification are three different kinds of evidence; only the on-device link satisfies a stage's gate.
6. Stage 2's `SerialTransport` uses a bounded queue of 64 entries; HTTP `202 Accepted` only means the event was queued, never pretending the device has processed it. When a serial write fails, the current frame is kept, the device is rediscovered by `303A:1001`, and the write is retried.
7. No scraping the Codex UI, and no parsing unstable transcripts. V1 uses only the official hooks; the adapter scripts drop prompts, transcripts and tool content before they cross the process boundary. A task card's session name comes from records the two apps already keep locally (the `title` in the Claude app's session index, and `name` and `git_branch` from `threads` in Codex's `state_5.sqlite`); these are the names already shown in each app's sidebar, not prompts. Codex's `title` column holds the raw first message and is never read. Lookups are cached: hooks arrive every few seconds, so we can't scan directories or open a database each time.
8. Task cards are ordered by most recent activity, at most 3; the global expression's priority is "needs input > working". Animation and voice assets stay in the firmware; the Mac only sends state snapshots.
9. The hook lifecycle doesn't guarantee a closing event, so activities must be able to expire on their own. Working expires after 30 minutes, needs input after 4 hours. The limits differ because silence during working usually means the process is gone, while silence during needs input only means the user hasn't come back yet. Cleanup is driven by a background sweep every 60 seconds; it can't be triggered lazily only when a new hook arrives.
10. One adapter per agent, but all adapters write into the same aggregator. The device has one screen and one buddy; two aggregators would each maintain their own task card stack and overwrite each other. So adapters hold no state and only translate events and synthesize activity identities. Task card titles carry an agent prefix, because the two agents often work in the same directory.
11. CI and agents share the same task cards and announcements; only the source differs: `vibebuddyd` actively polls GitHub Actions instead of relying on hooks. A run is announced when it finishes only if the daemon actually saw it running; otherwise every daemon restart would re-announce each repo's most recent historical result. Which repos to watch needs no user configuration: hooks already carry `cwd`, the projects an agent has recently worked in are exactly the ones whose CI you care about, and the repo name is derived from the local `.git/config`.
12. The source K2 opens must follow the same priority as the screen's main state; no separate "recent windows" list. The aggregator stores only what it needs to locate the source (thread id, session id or repo/run id), never prompts or replies; external arguments never pass through a shell. A Codex sub-agent's lifecycle session isn't a thread the desktop app can navigate to, so the hook must map it to the parent thread before associating a source. Where K2 lands depends on where the session runs: only a session running in the agent's own desktop app gets a deeplink; one running in another app brings that app to the front; a session with no host is skipped. Both the host and the desktop session id are determined by the hook from the process environment and reported up; the daemon only routes and never guesses from disk. Claiming a transcript by CLI `session_id` is one-to-many and would open shadow sessions.
13. Pomodoro, Leisure and mode switching all live in the firmware; `vibebuddyd` only receives diagnostic lines. They are device I/O and deterministic rendering, which are the firmware's job anyway. More importantly, when a user relies on it as a timer the Mac side may not be running, and a timer that only ticks when it gets a heartbeat is useless. All firmware logic that doesn't touch hardware (the Pomodoro state machine, the Leisure director, drawing code, serial protocol handling) lives in `firmware-rs/core` and runs under `cargo test` on the Mac: the state machine and director have unit tests, the whole serial chain has end-to-end tests against a fake board, and screens and skits have a host preview, so layouts and animations can be seen before flashing.
14. The Mac GUI is a native SwiftUI menu bar app that supervises `vibebuddyd` as a helper; voice packs live in a separate partition and are written over the serial protocol; the hook becomes a single Rust binary copied into Application Support. See ADR-0004, 0003 and 0005 for these three trade-offs respectively, and [`app.md`](app.md) for the overall design.
15. Linux runs the same daemon and hook without an app, so the few jobs the app does on the Mac move to the smallest place that can hold them: systemd user units supervise the daemon, `vibebuddy-hook install` writes the hook config with the same rules as `HookConfig.swift`, and the daemon itself sends the link-lost notification through `notify-send` (on macOS it leaves that to the app). Files follow the XDG base directories. K2 keeps decision 12's split: with no bundle ids on Linux, the hook reports its ancestor pids (read from `/proc`, spawning nothing), and when K2 is pressed the daemon asks Hyprland for the window owned by the nearest of them and focuses it by address. No window means headless, as with tmux or SSH.
16. The Linux app (`desktop/`, binary `vibebuddy-desktop`) is written in Rust with iced, with a StatusNotifierItem tray through `ksni`, so it stays in the same workspace and language as the daemon and could later run on other platforms; the Mac keeps its SwiftUI app. Unlike the Mac app it doesn't supervise the daemon, which systemd already does, so quitting it leaves the box online. It holds as little logic as it can: status comes from the daemon's API, hook config is written by `vibebuddy-hook install`, and its UI strings use the Mac app's English keys, so both apps share one zh-Hans table, checked by `tools/check-localization.py`. On Omarchy it reads the current theme's `colors.toml` and switches when the theme does.
17. A box is written anything but `device.hello` only once it reports a build (`DISPLAY READY BUILD …`, at boot or in answer to hello), and it is written nothing at all, hello included, while it might run other firmware. Muse, on a box that runs it, enumerates as the same `303A:1001`, and its console reads the letters of our JSON as key presses: heartbeats scrolled its menu, recorded voice notes and twice reset its pairing. So a newly connected or just reset box is first listened to for 6 seconds: our build ends it at once (opening the native port resets the chip, and our firmware reports its build about two seconds later), and an ESP-IDF application log line (`I (1234) tag: …`, which our firmware never prints and Muse prints every 5 seconds) means other firmware. Only a box that stays quiet gets one hello, and no build within 5 seconds of it means other firmware too. A box running other firmware gets nothing written and no probes, and its output isn't read as ours, until the ROM's `ESP-ROM:` banner says it reset; the judgment comes only from what an open port prints, never from opening ports to probe them. With several boxes plugged in, discovery connects to the one whose USB serial number it remembered when that box last reported our build.
