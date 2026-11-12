# VibePlay

VibePlay is our game handheld that runs QingOS. It also shows the buddy (ADR-0025). It gets the same task cards, today's stats and announcements as the box, over Wi-Fi. It does not take the link: a computer still links to one box, and the box and a VibePlay work at the same time.

The first handheld is the Anbernic RG 34XXSP: a 720×480 screen, a D-pad, A B X Y, L1 R1, START, SELECT, a speaker, and Wi-Fi. It has no USB device port we can use and no microphone. On QingOS the VibePlay is a full-screen app; its code is in [`vibeplay-app/`](../vibeplay-app/).

This document is the contract between three pieces of work: the daemon and the two apps (this repo), the VibePlay app (this repo, `vibeplay-app/`), and QingOS (longzhi/qingos: the capability to accept connections and to answer mDNS). Change it before you change any of them.

## Words

- **VibePlay (掌机)**: the device and the app on it. Not a box: it never holds the link.
- **VibePlay key**: an Ed25519 key the VibePlay app makes the first time it runs, from the system's random source, and keeps in its own data (`data_write("identity", …)`). Written as unpadded base64url, 43 characters, like a box key.
- **Pairing code (配对码)**: six digits the VibePlay shows while it waits to be paired.

## Finding the VibePlay

The VibePlay is the server, as the box is on the local network. The computer finds it and connects.

- The VibePlay accepts TCP on port **7342**.
- It answers mDNS (DNS-SD) for **`_vibebuddy-vibeplay._tcp`** on port 7342. The instance name is `VibePlay <first 8 characters of the VibePlay key>`. The TXT record has `key=<VibePlay key>`, `name=<device name>` and `paired=0` or `paired=1`. QingOS gives an app no device name, so the app uses the instance name as `name`, here and in `vibeplay.hello`. The app calls `mdns_set` as it starts; until then QingOS answers with the app's name (`vibebuddy`) and no TXT, and it withdraws that answer when the app's own comes.
- The service type is not `_vibebuddy._tcp`, so the daemon's box discovery never picks up a VibePlay, and older daemons never see one.
- The daemon browses `_vibebuddy-vibeplay._tcp` all the time. It remembers the last address of each paired VibePlay, as it does for the box (`box-address`). `VIBEBUDDY_VIBEPLAY_ADDR=host:port` fixes one address and turns discovery off, for QEMU and tests. With no TXT record there, the daemon connects once to read the key from `link.noise`, and closes; it names that VibePlay `VibePlay <first 8 characters of the key>` until its `vibeplay.hello` gives a name.

## Pairing without a cable

The box pairs over USB, because the cable proves the person is at the box. The VibePlay has no cable, so the person proves it by reading a code off the VibePlay's screen.

1. A VibePlay with no paired computer shows a pairing code and how to use it: open VibeBuddy on the computer, pick this VibePlay and type the code. A VibePlay that has computers already shows one when the person picks **PAIR** in its menu. The code is six random digits. The VibePlay makes a new code after three wrong tries, and after 10 minutes.
2. The App lists every VibePlay the daemon has found that isn't paired with this computer, by `name`, whatever its TXT `paired` says: a VibePlay paired with another computer can pair with this one too. The person picks one and types the code.
3. The daemon connects. The VibePlay sends `link.noise` first, as on every connection (see below), and the computer checks that its key is the `key` in TXT. The computer then sends `{"version":1,"event":"vibeplay.pair_start"}` in the clear, so the VibePlay knows that a pairing handshake follows and not a link.
4. They run **`Noise_XX_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibeplay-pair-v1`, computer as initiator, one handshake message per line as unpadded base64url, all three with empty payloads. Both ends use their Ed25519 keys converted to X25519, as for the box link (`noise_link.rs`). The computer checks that the VibePlay's static key is the `key` in TXT.
5. Inside the session the computer sends `{"version":1,"event":"vibeplay.pair","code":"123456","name":"dragon's MacBook"}`.
6. The VibePlay answers `{"version":1,"event":"vibeplay.paired"}` and keeps the computer's key and name, or answers `{"version":1,"event":"vibeplay.pair_error","reason":"code"}` (wrong code), `"reason":"full"` (four computers already) or `"reason":"closed"` (it shows no code now). It closes the connection either way. The computer's key it keeps is the X25519 static key Noise showed it, which is what the link's handshake shows it again; it never needs the Ed25519 form.
7. The daemon keeps the VibePlay's key, name and last address in `vibeplays.json` in its state directory, and connects again as below.

A VibePlay keeps up to four paired computers. **FORGET ALL** on the VibePlay forgets them all; "Forget" in the App forgets one VibePlay on that computer and sends `{"version":1,"event":"vibeplay.unpair"}` if it is connected; the VibePlay then forgets that computer and closes.

**What this does not stop.** An attacker on the same network who is in the middle at the moment of pairing could pass the code along and pair with both ends. The code is checked once, online, with three tries, so guessing does not work. A VibePlay only shows status and asks to open a window, so we accept this for now. Approving an Agent's request from the VibePlay (not in this version) needs a stronger pairing first.

## The connection

After pairing, the computer connects with **`Noise_IK_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibeplay-v1`, computer as initiator, knowing the VibePlay's key. Framing is the box's: one line per handshake message, then one Noise transport message per line, each the ciphertext of one VibeBuddy Protocol line, as unpadded base64url (`protocol.md`, "Local network link"). Both handshake messages have empty payloads.

- The VibePlay sends `{"version":1,"event":"link.noise","vibeplay":"<VibePlay key>"}` in the clear first, on every connection, as the box does with `box`. A first line from the computer that starts with `{` is `vibeplay.pair_start` (above); anything else is the first IK message.
- A computer that is not paired gets `{"version":1,"event":"link.error","code":"auth"}` in the clear, and the VibePlay closes. A daemon that gets it forgets that VibePlay on its side too, so the App offers it for pairing again: the person forgot the computers on the VibePlay.
- The VibePlay talks to one computer at a time. A second paired computer gets a sealed `link.busy` with `reason` `linked` and `owner`, and is closed. The first one keeps it until it goes away; a computer that has been silent for 15 s is gone. There is no takeover: the VibePlay is not shared the way the box is.
- After the handshake the VibePlay sends `{"version":1,"event":"vibeplay.hello","build":"…","name":"…","screen":[720,480]}`.
- The daemon connects to every paired VibePlay it can reach, at the same time as it links to a box.
- **Connecting again.** QingOS closes the app's port, ends its connections and withdraws its mDNS answer whenever the app stops: another app in front, the menu key (to change the volume, say), the lid. It opens them again when the app is in front again. So a lost VibePlay is normal, not a fault: the daemon tries it again 2 s after the connection ends, then every 2 s, and 15 s after a `link.busy`. It needs no takeover time. On each connection the daemon sends a heartbeat and the screen as it is now (a state event with `suppress_audio`), so the VibePlay catches up without speaking. Connecting is not an announcement: it never brings the daily greeting or a welcome back, which follow the Agents' activity, not the VibePlay.

## What the computer sends

The same lines it sends the box, built from the same snapshot (`activity.rs`, `occasions.rs`), with these differences:

- **State events** (`task.start`, `agent.input_required`, `task.done`, `task.error`, `agent.blocked`, `agent.idle`) with `tasks`, `tasks_total`, `stats`, `announcement`, `announcement_id`, `occasion` and `suppress_audio`, exactly as for the box.
- **`buddy.say`** for the daily greeting, as for the box.
- **`device.heartbeat`** every 5 s, with `build`, `hour` and `day`.
- **`quiet`**: every state event and `buddy.say` carries `"quiet":true` while a box is linked to this computer, and `"quiet":false` while none is. A quiet VibePlay shows everything and plays nothing. The daemon decides, because only it knows whether a box is linked (dragon, 2026-11-11: when both are there, the box speaks).
- **The occasion is shared.** The daemon picks one occasion per announcement and sends it to the box and the VibePlay alike, so "first done of the day" is counted once. The VibePlay gets the very line the box gets, plus `quiet`.
- **Not sent**: anything that looks after a box (`device.*` except the heartbeat, `xfer.*`, Character packs, Wi-Fi, Relay, screens, decorations) and voice commands (`voice.*`). The VibePlay ignores any event it does not know.

## What the VibePlay sends

- `{"version":1,"event":"button","button":"K2","action":"press"}` when A is pressed on the duty screen. The daemon does what it does for the box's K2.
- `{"version":1,"event":"focus","task":"<id>"}` when a card is picked with the D-pad and A. The daemon does what it does for a tapped card.
- Nothing else in this version.

## On the VibePlay

- **Screens**: the box's screens, pixel for pixel, in both of its looks: the robot (the default, as on a new box) and korekore. The app links the firmware's drawing code (`firmware-rs/core`: `Display`, `Pomodoro` and `Leisure`) and gives it the state events as the box's firmware does. So on duty (state, up to 6 cards on 2 pages, stats), needs input, done, failed, no link, the Pomodoro and the leisure skits are the box's own; done goes back to the cards after 5 s. The firmware draws 320×240 RGB565. The app doubles it to 640×480 and gives it to QingOS as a `Frame565` (`frame = "640x480"` in `app.toml`, QingOS ADR 0036), which shows it in the middle of the 720×480 screen with 40 black columns each side. The app gives a frame only when it changed. While the computer talks, the link icon is the box's LAN icon. The words are in English capitals, as on the box. The screens that only the VibePlay has (the pairing code and looking for a computer) are drawn with the firmware's canvas and fonts, one for each look, and the menu is the box's menu panel. Screenshots of every screen in both looks: [`design/vibeplay/screenshots/app/`](design/vibeplay/screenshots/app/).
- **Why the box's code**: the first app drew its own copy at 240×160 with a 256-colour palette, because that was all a full-screen QingOS app could draw. The copy did not look like the box (the robot's face, the antenna's colour, the layout), and each change to the box's screens had to be made again for it. A 640×480 RGB565 frame (QingOS ADR 0036) holds the box's 320×240 screen at exactly twice its size, so the app now runs the firmware's own code. The costs: the wasm is bigger (about 540 KB), and a change to the box's drawing code also changes the VibePlay's screens. `vibeplay-app/tools/compare-box-screens.sh`, which CI runs, compares the app's screens with the firmware's previews. The firmware has one change for the VibePlay: the key names in its hints can be set, so the VibePlay's say START, HOLD START, SELECT and A where the box's say K0, HOLD K0, K1 and K2.
- **Keys**: on duty, A sends `button` K2. Up and down pick a card (it gets the box's outline, and the page turns to it); A then sends `focus` for it, and B (or 8 s) lets go of it. X opens the menu; B closes it and leaves the pairing screen of a VibePlay that has computers. Any key brings the buddy back from its leisure skits.
- **Menu**: **PAIR** (shows a new code, for a second computer and up), **FORGET ALL** (asks first), **SCREENS** (ROBOT or KOREKORE, kept on the VibePlay) and **MUTE**.
- **Sound**: the robot's lines for `input_required`, `done` and `failed`, packed into the app as WAV (`vibeplay-app/assets/`, made from `voices/jessica/` by `vibeplay-app/tools/make-sounds.py`). The app plays them with `tone_wav`, which takes at most 2 s, so they are 16 kHz mono; the longest is 1.5 s. It picks the line as the box does: by the event, none with `suppress_audio`, and `announcement` first. Not when `quiet` is true, the VibePlay is muted in the menu, or QingOS's mute is on. Other Characters come later.
- **Pomodoro**: runs in the app with the firmware's code, as on the box: 25 minutes of focus and 5 of break, each started by START. START starts, pauses and goes on, and shows the Pomodoro; holding START gives up. SELECT switches between on duty and Pomodoro. A phase that ends plays a short chime (the robot's voice has no line for it short enough), unless muted. Today's count is kept on the VibePlay. The computer never hears of it.
- **Lid**: closing the lid stops the app (QingOS ADR 0030). When it opens, the app waits for the computer to connect again and shows the state it sends. A VibePlay does not remind anyone while it is closed (dragon, 2026-11-11).
- **No link**: after 15 s with no line from the computer, the buddy closes its eyes and the screen turns gray, as on the box; the cards stay. A connection that closes is not enough on its own, since the daemon connects again within 2 s.

## The daemon's API

- `GET /v1/status` gets `vibeplays`: one item per VibePlay found or paired, with `key`, `name`, `paired` (with this computer), `connected` and `address`. Paired ones come first.
- `POST /v1/vibeplays/pair` with `{"key","code"}`: pairs. Answers 200, or 400 with `{"error":"code"}`, `{"error":"full"}`, `{"error":"closed"}` or `{"error":"unreachable"}`.
- `POST /v1/vibeplays/forget` with `{"key"}`: forgets one VibePlay. Answers 200, or 404 when it isn't paired.

## The apps

Mac and Linux, in the same PR (AGENTS.md):

- The Device page gets a **VibePlays** part: VibePlays found and not paired, each with **Pair…**, which asks for the six digits; paired VibePlays, with connected or not and **Forget**, which asks first. A pairing that fails says why, from the API's `error`: a wrong code, four computers already, no code on the screen (choose PAIR on the VibePlay), or not reached. Daemons older than this report no `vibeplays`, and then the part isn't shown.
- zh-Hans strings on both sides: 掌机、配对…、输入掌机上显示的六位数、忘掉这台掌机.

## Testing without the VibePlay

- `uv run tools/simulate-vibeplay.py` answers on port 7342 as a VibePlay: it shows a pairing code, pairs and links as above, prints what it gets, and sends A (`a`) and `focus` (`f <task id>`) from the keyboard. With `VIBEBUDDY_VIBEPLAY_ADDR=127.0.0.1:7342` the daemon talks to it; with `--mdns` it also answers mDNS through `dns-sd` (macOS) or `avahi-publish` (Linux), so discovery is tested too. `--state FILE` keeps its key and computers between runs.
- `vibeplay-app/` has its own tests: `cargo test --workspace` there checks pairing, the link and the events against snow (the daemon's Noise library), the keys and the menu with QingOS's test doubles, and draws every screen; with `VIBEPLAY_SHOTS=<folder>` it saves them as PNGs. `tools/compare-box-screens.sh` there compares the screens with the firmware's previews, and `tools/compare-wasm-shots.sh` compares the release wasm's `vibeplay shot`s with the tests' screens. The daemon's test `the_vibeplay_app_pairs_and_links_with_this_daemon` runs the daemon's own pairing and link against the app's core.
- `vibeplay run` in `vibeplay-app/` plays the app in a window on port 7342, with a vibeplay CLI from vibeplay-sdk 9e8e424 or later (older ones have no 640×480 frames); then `VIBEBUDDY_VIBEPLAY_ADDR=127.0.0.1:7342` connects a daemon to it.
- QEMU's user network does not pass multicast, so mDNS is checked on the real handheld only. In QEMU, forward port 7342 and use `VIBEBUDDY_VIBEPLAY_ADDR`.
