# VibeBuddy Protocol v1

VibeBuddy Protocol is the application protocol between `vibebuddyd` and a VibeBuddy device. v1 uses UTF-8 NDJSON; the transport is responsible for delivering the byte stream reliably, and the protocol doesn't depend on any particular serial port name.

## Framing

- Each message is one JSON object terminated by a single LF (`\n`).
- Receivers also accept CRLF (`\r\n`), but senders always use LF.
- Blank lines are ignored.
- A single JSON message is at most 1024 bytes, excluding the line terminator. Anything longer is dropped as a whole line and answered with `ERROR input_too_large`.
- v1 doesn't support JSON spanning multiple lines, JSON arrays, or multiple JSON objects on one line.

## Envelope

Every message must include:

- `version`: an integer; in v1 it must equal `1`.
- `event`: a non-empty string.

Fields such as `id`, `title` and `message` are used by specific events. Stage 1 doesn't require `id`, so the Hello below is a valid message:

```json
{"version":1,"event":"task.done","title":"Hello"}
```

Receivers ignore unknown fields, so older firmware accepts newly added optional fields. An unknown `event` must still go through framing and envelope parsing; Stage 1 firmware prints its name and doesn't break the connection just because the event isn't implemented yet. An unrecognized `version` gets `ERROR unsupported_version` and must not be handled as a best guess at v1.

## Direction

The first batch of Mac-to-device events:

- `task.start`
- `task.done`
- `task.error`

What the current firmware recognizes:

- `task.start`: working; no voice.
- `agent.input_required`: needs user input; plays the needs-input line ("Hey, I need you for a sec." in the built-in voice) once.
- `task.done`: done; plays the done line ("All done!") once; after 5 seconds it goes back to the cards in `tasks` that are still open, or to idle when none are.
- `task.error` / `agent.blocked`: failed; plays the failed line ("Uh-oh, something went wrong.") once.
- `agent.idle`: back to idle; no voice.
- `device.heartbeat`: proves the link is alive; not displayed, no diagnostic line echoed, no voice.

`vibebuddyd` may attach a `tasks` array of up to 3 items. The array is in reverse order of most recent activity, and the device draws task cards in the order given:

```json
{"version":1,"event":"task.start","title":"GAMMA","tasks":[{"title":"GAMMA","status":"working","elapsed_s":75},{"title":"BETA","status":"input_required","elapsed_s":900}],"stats":["7 DONE","4 ASKS","1H23 BUSY"]}
```

Each item contains `title`, `status` and `elapsed_s`, with optional `project`; the current status values are `working`, `input_required`, `done` and `failed`. `title` is the card's first line: the name the agent gave the session itself (the Claude app's session title, Codex's thread name) or the branch name, falling back to the project name only when neither exists. `project` is the project name, which the device draws on the second line unless it duplicates the title. Older firmware ignores `tasks` and `project` under the v1 rules. Future events may include `task.progress`, `task.cancelled`, `agent.waiting`, `message`, `system` and `device.status`.

`elapsed_s` is the number of seconds since the activity entered its **current state**, not since the previous event: a working card answers "how long has this turn been running", and a card waiting for input answers "how long has it been waiting". The device keeps counting on its own after receiving it, because `vibebuddyd` deduplicates and sends nothing while the visible state is unchanged, yet the number on screen has to keep ticking. And precisely because of deduplication, `elapsed_s` and `stats` are stamped onto the event only after deduplication; putting them in the snapshot would turn every tool event into a redraw, constantly knocking the working animation back to its first frame.

`stats` holds up to 3 lines of today's stats, which the idle screen cycles through. It's sent with every state event, not just `agent.idle`: after `task.done` the device returns to idle on its own, and that is exactly the moment the user glances over, so the cached stats must already include the task that just finished. Counts reset at each local calendar day and are persisted to `~/Library/Application Support/VibeBuddy/stats.json`; otherwise every daemon restart would zero the numbers that the screen labels as "today".

When a background task finishes but the screen still needs to show other active tasks, `vibebuddyd` attaches `"announcement":"done"` to the current state event. This is a one-shot voice notification that doesn't change the screen state; `announcement_id` identifies the corresponding turn. When the device receives it, it queues the done line to play once. Failures take the same path with `"announcement":"failed"`, playing the failed line once.

A done or needs-input announcement may carry `"occasion"`, naming a special occasion that changes only which pool of the Character's lines the box draws from: `first_done`, `milestone` or `late_night_done` on a done, `late_night_input` on a needs input (see [`characters.md`](characters.md)). The device ignores an occasion that doesn't belong to the announcement, and older firmware ignores the field and plays the ordinary line.

The first time the link comes up on a local calendar day, once the firmware has reported its build, `vibebuddyd` sends the daily greeting. It changes nothing on screen and isn't agent activity; only the three greetings may be said this way:

```json
{"version":1,"event":"buddy.say","occasion":"greeting_morning"}
```

When a change in aggregated tasks only requires redrawing an existing waiting-for-input state, `vibebuddyd` attaches `"suppress_audio":true`. The device keeps showing `agent.input_required` but doesn't replay the alert it already played. A one-shot `announcement` takes precedence over this field.

`vibebuddyd` sends a heartbeat every 5 seconds, carrying its own build identifier, the local hour and the local date:

```json
{"version":1,"event":"device.heartbeat","build":"9b642af 2026-09-14 17:41","hour":14,"day":20260915}
```

`build` is `git describe --always --tags --dirty` plus the binary's timestamp. It's repeated with every heartbeat instead of sent once in a handshake, because the device can restart at any time and a one-time handshake would get lost. The device redraws only when the value changes; otherwise it would refresh the screen every 5 seconds. `hour` is the Mac's local hour, which Leisure mode uses to tell day from night; `day` is the local date as YYYYMMDD, which resets Pomodoro's daily record. The device has no clock, and shouldn't connect to Wi-Fi just for this. When an older daemon doesn't send these fields, the device assumes daytime and never rolls over the day.

When writing to the BOX's CH343 UART bridge, the Mac writes in segments paced to line speed (waiting for each 128 bytes to drain before writing the next): this bridge can't take more than about two hundred bytes of continuous data at once, and it scrambles the content while keeping the length unchanged. The native USB port gets whole frames.

The device only displays the build identifier; it doesn't compare it with its own firmware identifier. The two sides ship on different schedules anyway, so treating a mismatch as a warning would only produce a constant stream of false alarms. When the field is missing the device shows `?`, which means the other end is an older daemon that doesn't send it yet.

If the device receives **no** message of any kind for more than 15 seconds, it considers the link lost, overlays `NO LINK` and turns the screen gray; any valid line restores it. The check is based on all messages, not just heartbeats, because when things are busy the real events alone prove the link is alive. The firmware doesn't print an `EVENT` diagnostic line for heartbeats; otherwise it would produce tens of thousands of log lines a day.

Device-to-Mac events use NDJSON as well, for example:

```json
{"version":1,"event":"button","button":"K2","action":"press"}
```

Currently only a K2 short press is reported, in every mode. K0 (Pomodoro), K1 (switch mode; long-press for the device menu, see [`device-menu.md`](device-menu.md)), a K2 long press (mute) and every key while the menu is open are consumed by the firmware itself and not reported. A short press counts only on release, because only on release do you know it wasn't the start of a long press, so reporting is deferred from press to release. See [`pomodoro.md`](pomodoro.md) for modes and Pomodoro, and [`leisure.md`](leisure.md) for Leisure. On receiving it, `vibebuddyd` picks a destination in this order: first, a task waiting for someone to answer (it's what the screen's main state shows, and it's blocking a person); next, the task most recently announced as finished (it has left the card stack and can no longer be seen on screen, whereas a task still running stays on screen and never needs K2 to find it); only then the newest work item. The destination is replaced by the next announcement, with no time window: the whole point of this device is that you're away from the computer, and expiring the destination on a wall clock would assume the user is always sitting nearby. Once an activity is chosen, where it runs decides what happens: only if it runs in the agent's own desktop app is a deeplink used (a top-level Codex activity opens its own thread, a sub-agent activity opens the parent thread that owns it, and Claude Code uses the desktop session id reported by the hook via `claude://code/continue?session=<desktop session id>`); if it runs in another app (a terminal, an editor's integrated terminal), that app is brought to the front, using a bundle id reported by the hook, so the daemon doesn't need to know the app; a session with no host (SSH, a background process) is skipped outright and the next candidate is tried. GitHub Actions opens the corresponding run. When there's no current activity, it falls back to the most recent locatable source; that destination is written to a local state file, so it survives a daemon restart. Candidates are tried one by one in this order: a Codex thread is first checked against the local thread table, and skipped if it isn't there, so a blank session doesn't get opened. K0, K1 and long-press/release are not yet bound.

The `READY`, `EVENT`, `TITLE` and `ERROR` lines the firmware prints are diagnostic text for verifying the on-device link, not formal device-to-Mac JSON events. In the same category are `MODE DUTY` / `POMODORO` / `LEISURE` (mode switches); `POMODORO FOCUS START` / `FOCUS END` / `BREAK START` / `BREAK END` / `PAUSED` / `RESUMED` / `STOPPED` / `BREAK SKIPPED` (Pomodoro transitions); `LEISURE ALERT` / `BORED` / `SLEEPY`, `LEISURE SKIT <name>`, `LEISURE LIGHTS OUT` / `ON` (Leisure); and `CLOCK HOUR <n>` (the received hour changed), `TALLY LOADED <count> <seconds>S DAY <date>` (the daily record restored at boot), `MUTE ON` / `OFF` (K2 long press, or the menu), `MENU OPEN` / `MENU CLOSED` (the device menu) and `AUDIO MUTED <which line>` (a voice line swallowed while muted). Pomodoro and Leisure state lives only in the firmware; the Mac just logs it.

## Device maintenance: blink to identify and voice pack writes

These messages are the app operating on the device itself. They don't count as agent activity, don't wake Leisure mode, and don't produce `EVENT` diagnostic lines.

`device.hello` is the Mac's greeting right after connecting, sent once the box has been listened to and hasn't given itself away as other firmware (see decision 17 in [`architecture.md`](architecture.md)); a box that runs other firmware is written nothing, hello included, and `/v1/status` reports `device.foreign_firmware: true`. It is needed because mode, firmware build number, voice and volume are otherwise reported only at boot or on change, and the daemon restarts more often than the device, so without asking it would never know. The device replies with five diagnostic lines: `DISPLAY READY BUILD …`, `FIRMWARE VERSION …`, `MODE …`, `VOICES …`, `VOLUME …`; firmware that can pair adds `BOX KEY …` after the version (see Pairing below). `FIRMWARE VERSION` carries the firmware version (`0.2.2`), which orders releases (ADR-0010); it is also printed at boot, and firmware older than it doesn't print it.

`device.echo` is a link self-test: the device returns the length and CRC32 of the `data` string as `{"event":"echo","length":…,"crc":…}`, then echoes a line `ECHO …` verbatim. It's used to check whether the serial port is receiving corrupted bytes; that's how the UART bridge problem was tracked down.

`device.identify` makes the device's backlight flash rapidly for about a second, visible in any mode; onboarding uses it to confirm which box is connected. The device replies with one diagnostic line, `IDENTIFY`.

`device.volume` sets the speaker volume: `level` is the codec's 20 to 100, out-of-range values are clamped, and the value is stored in the device's NVS so it survives restarts; without `level` it's just a query. After applying it the device replies with a line `VOLUME <n>`, the same line it prints unprompted when the volume is changed from the device menu (a board without a codec replies `VOLUME ERROR` and then reports the current value); with `preview: true` it also plays the done line at the current volume, which stays silent while muted just like any other announcement. The floor is above zero: muting happens only by long-pressing K2 on the device, and is deliberately not persisted.

Character packs (format in `firmware-rs/core/src/character_pack.rs`), and the older voice packs (`firmware-rs/core/src/voice_pack.rs`), are written into the `voices` partition over the same serial link, without resetting and without esptool; both connection types behave the same. Stop-and-wait flow control: after each chunk the Mac waits for the device's acknowledgment, and sends nothing further without one.

```json
{"version":1,"event":"voice.begin","size":1523456}
{"version":1,"event":"voice.chunk","seq":0,"crc":305419896,"data":"<base64, at most 672 bytes of raw data>"}
{"version":1,"event":"voice.end"}
```

The device's acknowledgments:

```json
{"version":1,"event":"voice.ready","seq":-1}
{"version":1,"event":"voice.ack","seq":0}
{"version":1,"event":"voice.written","voice":"wanwanxiaohe"}
{"version":1,"event":"voice.error","seq":12,"message":"ESP_ERR_INVALID_CRC"}
```

`begin` first waits for any line currently playing to finish, then erases the needed range; `size` is the whole pack's byte count, including the header: 1024 bytes for a Character pack, 256 for a voice pack, told apart by the magic in the first chunk. Chunks must arrive consecutively by `seq` starting from 0, each with at most 672 bytes of raw data, so the whole line after base64 still fits within the 1024-byte limit. `crc` is the CRC32 of the chunk's raw bytes (the same as zlib's); the device checks it right after decoding and, on a mismatch, replies `voice.error` and aborts rather than waiting until the end. The header stays in device memory; on `end` the device reads the flash back to verify the payload CRC, writes the header only if that passes, then switches to the new Character. If any step fails it replies `voice.error` and abandons the session; the firmware then sees the partition as empty and announcements fall back to the built-in voice. Announcements during a write use the built-in voice.

## Pairing

A paired computer may later link to the box over the local network or through the Relay (ADR-0012, [`relay-protocol.md`](relay-protocol.md)). Pairing happens over USB only: the cable is the proof that whoever pairs is at the box. Keys are Ed25519 public keys, 32 bytes as unpadded base64url (43 characters).

The box makes its own key the first time it boots firmware that can pair, from hardware entropy, and keeps it, with the computers paired to it, in the `link` partition, which flashing firmware never touches. It reports the key at boot and in answer to `device.hello` as `BOX KEY <key>`; that line is also how the daemon knows the firmware can pair. A box without the `link` partition (the single-slot layout) prints no `BOX KEY` and answers every pairing message with `PAIR UNAVAILABLE`.

Whenever `vibebuddyd` sees `BOX KEY`, it pairs: its own key lives in `identity` next to `config.json`, and `name` is the computer's name. Pairing again is harmless; the box only writes flash when something changed.

```json
{"version":1,"event":"device.pair","key":"<computer key>","name":"dragon's MacBook"}
{"version":1,"event":"device.unpair","key":"<computer key>"}
{"version":1,"event":"device.pairs"}
```

Replies are diagnostic lines. `device.pair` gets `PAIRED <computer key>`, or `PAIR FULL` once 16 computers are paired. `device.unpair` gets `UNPAIRED <computer key>`. `device.pairs` gets `PAIRS <n>` followed by one `PAIR <computer key> <name>` per computer. A malformed key gets `PAIR INVALID`. Names longer than 32 bytes are cut. None of these is agent activity. When the box's own menu forgets every computer (COMPUTERS, [`device-menu.md`](device-menu.md)), it prints `UNPAIRED ALL`.

After pairing, the daemon asks `device.pairs`, and `/v1/status` reports `device.box_key`, `device.computer_key` (this computer) and `device.paired_computers`. `POST /v1/device/unpair` with `{"key": …}` forgets another computer; this computer is refused, since it pairs again whenever it's plugged in.

## Wi-Fi network

Set only over USB, like pairing. The box keeps the network in the `link` partition and reports only its name: at boot, in answer to hello, and on change as `WIFI NETWORK <ssid>`. The password never leaves the box.

```json
{"version":1,"event":"device.wifi.set","ssid":"Home","password":"…"}
{"version":1,"event":"device.wifi.forget"}
```

Replies: `WIFI NETWORK <ssid>`, `WIFI FORGOTTEN`, `WIFI INVALID` (an empty name, a name over 32 bytes, or a password over 63), or `WIFI UNAVAILABLE` (no `link` partition).

## Local network link

Over the local network the box is the TCP server and the computer connects. The bytes are the same NDJSON as over USB, after a handshake in which both ends prove their keys (`firmware-rs/core/src/lan.rs`):

```json
{"version":1,"event":"link.challenge","box":"<box key>","nonce":"<32 bytes>"}
{"version":1,"event":"link.hello","key":"<computer key>","sig":"<signature>","nonce":"<32 bytes>","takeover":false}
{"version":1,"event":"link.welcome","sig":"<signature>"}
```

The computer signs `vibebuddy-lan-v1\ncomputer\n<box key>\n<box nonce>`, and the box signs `vibebuddy-lan-v1\nbox\n<box key>\n<computer nonce>`. Keys, nonces and signatures are unpadded base64url. The computer checks the box's key against the `BOX KEY` it learned over USB, so it won't drive another box. The box refuses with `link.error` (`auth`, `bad_message`, `timeout` after 10 s) and closes. If it is linked elsewhere, it sends `link.busy` with `reason` `usb` (a computer talked over the cable in the last 15 s) or `linked` (another computer has the box), and closes.

A box links to one computer at a time (ADR-0011). A paired computer gets the box when it is free, when it was the last one linked, or once the last one has been gone 10 minutes. `"takeover":true`, which the user asked for, always gets it unless the cable is in use. Whoever loses the box gets `link.unlinked` with `reason` `takeover`, `usb` or `unpaired`, and is closed. Pairing and Wi-Fi messages that come over the network are refused with `ERROR cable_only`.

There is no encryption on the local network: like the USB link, the bytes are readable by anyone who can see them, and the handshake only decides who may drive the box.

## Stage 1 error output

| Output | Meaning |
| --- | --- |
| `ERROR invalid_json` | The line isn't complete, valid JSON |
| `ERROR invalid_message` | The root isn't an object, or `version`, `event` or `title` is missing or misused |
| `ERROR unsupported_version` | `version` isn't `1` |
| `ERROR input_too_large` | A line exceeds 1024 bytes |
