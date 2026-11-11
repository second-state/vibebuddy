# VibeGame

VibeGame is our game handheld that runs QingOS. It also shows the buddy (ADR-0025). It gets the same task cards, today's stats and announcements as the box, over Wi-Fi. It does not take the link: a computer still links to one box, and the box and a VibeGame work at the same time.

The first handheld is the Anbernic RG 34XXSP: a 720×480 screen, a D-pad, A B X Y, L1 R1, START, SELECT, a speaker, and Wi-Fi. It has no USB device port we can use and no microphone. On QingOS the VibeGame is a full-screen app; its code is in [`vibegame-app/`](../vibegame-app/).

This document is the contract between three pieces of work: the daemon and the two apps (this repo), the VibeGame app (this repo, `vibegame-app/`), and QingOS (longzhi/qingos: the capability to accept connections and to answer mDNS). Change it before you change any of them.

## Words

- **VibeGame (掌机)**: the device and the app on it. Not a box: it never holds the link.
- **VibeGame key**: an Ed25519 key the VibeGame app makes the first time it runs, from the system's random source, and keeps in its own data (`data_write("identity", …)`). Written as unpadded base64url, 43 characters, like a box key.
- **Pairing code (配对码)**: six digits the VibeGame shows while it waits to be paired.

## Finding the VibeGame

The VibeGame is the server, as the box is on the local network. The computer finds it and connects.

- The VibeGame accepts TCP on port **7342**.
- It answers mDNS (DNS-SD) for **`_vibebuddy-vibegame._tcp`** on port 7342. The instance name is `VibeGame <first 8 characters of the VibeGame key>`. The TXT record has `key=<VibeGame key>`, `name=<device name>` and `paired=0` or `paired=1`. QingOS gives an app no device name, so the app uses the instance name as `name`, here and in `vibegame.hello`. The app calls `mdns_set` as it starts; until then QingOS answers with the app's name (`vibebuddy`) and no TXT, and it withdraws that answer when the app's own comes.
- The service type is not `_vibebuddy._tcp`, so the daemon's box discovery never picks up a VibeGame, and older daemons never see one.
- The daemon browses `_vibebuddy-vibegame._tcp` all the time. It remembers the last address of each paired VibeGame, as it does for the box (`box-address`). `VIBEBUDDY_VIBEGAME_ADDR=host:port` fixes one address and turns discovery off, for QEMU and tests. With no TXT record there, the daemon connects once to read the key from `link.noise`, and closes; it names that VibeGame `VibeGame <first 8 characters of the key>` until its `vibegame.hello` gives a name.

## Pairing without a cable

The box pairs over USB, because the cable proves the person is at the box. The VibeGame has no cable, so the person proves it by reading a code off the VibeGame's screen.

1. A VibeGame with no paired computer shows a pairing code and the words "Open VibeBuddy on your computer and pick this VibeGame". A VibeGame that has computers already shows one when the person picks **Pair a computer** in its menu. The code is six random digits. The VibeGame makes a new code after three wrong tries, and after 10 minutes.
2. The App lists every VibeGame the daemon has found that isn't paired with this computer, by `name`, whatever its TXT `paired` says: a VibeGame paired with another computer can pair with this one too. The person picks one and types the code.
3. The daemon connects. The VibeGame sends `link.noise` first, as on every connection (see below), and the computer checks that its key is the `key` in TXT. The computer then sends `{"version":1,"event":"vibegame.pair_start"}` in the clear, so the VibeGame knows that a pairing handshake follows and not a link.
4. They run **`Noise_XX_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibegame-pair-v1`, computer as initiator, one handshake message per line as unpadded base64url, all three with empty payloads. Both ends use their Ed25519 keys converted to X25519, as for the box link (`noise_link.rs`). The computer checks that the VibeGame's static key is the `key` in TXT.
5. Inside the session the computer sends `{"version":1,"event":"vibegame.pair","code":"123456","name":"dragon's MacBook"}`.
6. The VibeGame answers `{"version":1,"event":"vibegame.paired"}` and keeps the computer's key and name, or answers `{"version":1,"event":"vibegame.pair_error","reason":"code"}` (wrong code), `"reason":"full"` (four computers already) or `"reason":"closed"` (it shows no code now). It closes the connection either way. The computer's key it keeps is the X25519 static key Noise showed it, which is what the link's handshake shows it again; it never needs the Ed25519 form.
7. The daemon keeps the VibeGame's key, name and last address in `vibegames.json` in its state directory, and connects again as below.

A VibeGame keeps up to four paired computers. "Forget computers" on the VibeGame forgets them all; "Forget" in the App forgets one VibeGame on that computer and sends `{"version":1,"event":"vibegame.unpair"}` if it is connected; the VibeGame then forgets that computer and closes.

**What this does not stop.** An attacker on the same network who is in the middle at the moment of pairing could pass the code along and pair with both ends. The code is checked once, online, with three tries, so guessing does not work. A VibeGame only shows status and asks to open a window, so we accept this for now. Approving an Agent's request from the VibeGame (not in this version) needs a stronger pairing first.

## The connection

After pairing, the computer connects with **`Noise_IK_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibegame-v1`, computer as initiator, knowing the VibeGame's key. Framing is the box's: one line per handshake message, then one Noise transport message per line, each the ciphertext of one VibeBuddy Protocol line, as unpadded base64url (`protocol.md`, "Local network link"). Both handshake messages have empty payloads.

- The VibeGame sends `{"version":1,"event":"link.noise","vibegame":"<VibeGame key>"}` in the clear first, on every connection, as the box does with `box`. A first line from the computer that starts with `{` is `vibegame.pair_start` (above); anything else is the first IK message.
- A computer that is not paired gets `{"version":1,"event":"link.error","code":"auth"}` in the clear, and the VibeGame closes. A daemon that gets it forgets that VibeGame on its side too, so the App offers it for pairing again: the person forgot the computers on the VibeGame.
- The VibeGame talks to one computer at a time. A second paired computer gets a sealed `link.busy` with `reason` `linked` and `owner`, and is closed. The first one keeps it until it goes away; a computer that has been silent for 15 s is gone. There is no takeover: the VibeGame is not shared the way the box is.
- After the handshake the VibeGame sends `{"version":1,"event":"vibegame.hello","build":"…","name":"…","screen":[720,480]}`.
- The daemon connects to every paired VibeGame it can reach, at the same time as it links to a box.
- **Connecting again.** QingOS closes the app's port, ends its connections and withdraws its mDNS answer whenever the app stops: another app in front, the menu key (to change the volume, say), the lid. It opens them again when the app is in front again. So a lost VibeGame is normal, not a fault: the daemon tries it again 2 s after the connection ends, then every 2 s, and 15 s after a `link.busy`. It needs no takeover time. On each connection the daemon sends a heartbeat and the screen as it is now (a state event with `suppress_audio`), so the VibeGame catches up without speaking. Connecting is not an announcement: it never brings the daily greeting or a welcome back, which follow the Agents' activity, not the VibeGame.

## What the computer sends

The same lines it sends the box, built from the same snapshot (`activity.rs`, `occasions.rs`), with these differences:

- **State events** (`task.start`, `agent.input_required`, `task.done`, `task.error`, `agent.blocked`, `agent.idle`) with `tasks`, `tasks_total`, `stats`, `announcement`, `announcement_id`, `occasion` and `suppress_audio`, exactly as for the box.
- **`buddy.say`** for the daily greeting, as for the box.
- **`device.heartbeat`** every 5 s, with `build`, `hour` and `day`.
- **`quiet`**: every state event and `buddy.say` carries `"quiet":true` while a box is linked to this computer, and `"quiet":false` while none is. A quiet VibeGame shows everything and plays nothing. The daemon decides, because only it knows whether a box is linked (dragon, 2026-11-11: when both are there, the box speaks).
- **The occasion is shared.** The daemon picks one occasion per announcement and sends it to the box and the VibeGame alike, so "first done of the day" is counted once. The VibeGame gets the very line the box gets, plus `quiet`.
- **Not sent**: anything that looks after a box (`device.*` except the heartbeat, `xfer.*`, Character packs, Wi-Fi, Relay, screens, decorations) and voice commands (`voice.*`). The VibeGame ignores any event it does not know.

## What the VibeGame sends

- `{"version":1,"event":"button","button":"K2","action":"press"}` when A is pressed on the duty screen. The daemon does what it does for the box's K2.
- `{"version":1,"event":"focus","task":"<id>"}` when a card is picked with the D-pad and A. The daemon does what it does for a tapped card.
- Nothing else in this version.

## On the VibeGame

- **Screens**: not paired (the code, and the name the computer shows it by), looking for a computer, and on duty (state, cards, stats). Needs input, done and failed take the whole screen, as on the box; done goes back to the cards after 5 s. A full-screen QingOS app draws 240×160 with a 256-colour palette, and QingOS shows it three times as big on the 720×480 screen, so the app draws at 240×160. It does not copy the box's code: the layout and colours follow korekore's box screens, and the buddy is the robot, drawn by the app. Screenshots: [`design/vibegame/screenshots/`](design/vibegame/screenshots/).
- **Keys**: on duty, A sends `button` K2. Up and down pick a card; A then sends `focus` for it, and B (or 8 s) lets go of it. X opens the menu; B closes it and leaves the pairing screen of a VibeGame that has computers.
- **Menu**: **Pair a computer** (shows a code, for a second computer and up), **Forget computers** (asks first), and **Mute**.
- **Sound**: the robot's lines for `input_required`, `done` and `failed`, packed into the app as WAV (`vibegame-app/assets/`, made from `voices/jessica/` by `vibegame-app/tools/make-sounds.py`). The app plays them with `tone_wav`, which takes at most 2 s, so they are 16 kHz mono; the longest is 1.5 s. It picks the line as the box does: by the event, none with `suppress_audio`, and `announcement` first. Not when `quiet` is true, the VibeGame is muted in the menu, or QingOS's mute is on. Other Characters come later.
- **Pomodoro**: runs in the app, as on the box: 25 minutes of focus and 5 of break, each started by START. START starts, pauses and goes on, and shows the Pomodoro; holding START gives up. SELECT switches between on duty and Pomodoro. A phase that ends plays a short chime (the robot's voice has no line for it short enough), unless muted. The computer never hears of it.
- **Lid**: closing the lid stops the app (QingOS ADR 0030). When it opens, the app waits for the computer to connect again and shows the state it sends. A VibeGame does not remind anyone while it is closed (dragon, 2026-11-11).
- **No link**: after 15 s with no line from the computer, the buddy closes its eyes and the screen turns gray, as on the box; the cards stay. A connection that closes is not enough on its own, since the daemon connects again within 2 s.

## The daemon's API

- `GET /v1/status` gets `vibegames`: one item per VibeGame found or paired, with `key`, `name`, `paired` (with this computer), `connected` and `address`. Paired ones come first.
- `POST /v1/vibegames/pair` with `{"key","code"}`: pairs. Answers 200, or 400 with `{"error":"code"}`, `{"error":"full"}`, `{"error":"closed"}` or `{"error":"unreachable"}`.
- `POST /v1/vibegames/forget` with `{"key"}`: forgets one VibeGame. Answers 200, or 404 when it isn't paired.

## The apps

Mac and Linux, in the same PR (AGENTS.md):

- The Device page gets a **VibeGames** part: VibeGames found and not paired, each with **Pair…**, which asks for the six digits; paired VibeGames, with connected or not and **Forget**, which asks first. A pairing that fails says why, from the API's `error`: a wrong code, four computers already, no code on the screen (choose Pair a computer on the VibeGame), or not reached. Daemons older than this report no `vibegames`, and then the part isn't shown.
- zh-Hans strings on both sides: 掌机、配对…、输入掌机上显示的六位数、忘掉这台掌机.

## Testing without the VibeGame

- `uv run tools/simulate-vibegame.py` answers on port 7342 as a VibeGame: it shows a pairing code, pairs and links as above, prints what it gets, and sends A (`a`) and `focus` (`f <task id>`) from the keyboard. With `VIBEBUDDY_VIBEGAME_ADDR=127.0.0.1:7342` the daemon talks to it; with `--mdns` it also answers mDNS through `dns-sd` (macOS) or `avahi-publish` (Linux), so discovery is tested too. `--state FILE` keeps its key and computers between runs.
- `vibegame-app/` has its own tests: `cargo test --workspace` there checks pairing, the link and the events against snow (the daemon's Noise library), the keys and the menu with QingOS's test doubles, and draws every screen; with `VIBEGAME_SHOTS=<folder>` it saves them as PNGs. The daemon's test `the_vibegame_app_pairs_and_links_with_this_daemon` runs the daemon's own pairing and link against the app's core.
- `vibegame run` in `vibegame-app/` plays the app in a window on port 7342, with a vibegame CLI built against qingos-sdk 0ff2e32 or later (older ones have no `listen`); then `VIBEBUDDY_VIBEGAME_ADDR=127.0.0.1:7342` connects a daemon to it.
- QEMU's user network does not pass multicast, so mDNS is checked on the real handheld only. In QEMU, forward port 7342 and use `VIBEBUDDY_VIBEGAME_ADDR`.
