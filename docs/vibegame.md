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
- It answers mDNS (DNS-SD) for **`_vibebuddy-vibegame._tcp`** on port 7342. The instance name is `VibeGame <first 8 characters of the VibeGame key>`. The TXT record has `key=<VibeGame key>`, `name=<device name>` and `paired=0` or `paired=1`.
- The service type is not `_vibebuddy._tcp`, so the daemon's box discovery never picks up a VibeGame, and older daemons never see one.
- The daemon browses `_vibebuddy-vibegame._tcp` all the time. It remembers the last address of each paired VibeGame, as it does for the box (`box-address`). `VIBEBUDDY_VIBEGAME_ADDR=host:port` fixes one address and turns discovery off, for QEMU and tests.

## Pairing without a cable

The box pairs over USB, because the cable proves the person is at the box. The VibeGame has no cable, so the person proves it by reading a code off the VibeGame's screen.

1. A VibeGame with no paired computer shows a pairing code and the words "Open VibeBuddy on your computer and pick this VibeGame". The code is six random digits. The VibeGame makes a new code after three wrong tries, and after 10 minutes.
2. The App lists every unpaired VibeGame the daemon has found (`paired=0`), by `name`. The person picks one and types the code.
3. The daemon connects and runs **`Noise_XX_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibegame-pair-v1`, computer as initiator. Both ends use their Ed25519 keys converted to X25519, as for the box link (`noise_link.rs`). The computer checks that the VibeGame's static key is the `key` in TXT.
4. Inside the session the computer sends `{"version":1,"event":"vibegame.pair","code":"123456","name":"dragon's MacBook"}`.
5. The VibeGame answers `{"version":1,"event":"vibegame.paired"}` and keeps the computer's key and name, or answers `{"version":1,"event":"vibegame.pair_error","reason":"code"}` (wrong code) or `"reason":"full"` (four computers already). It closes the connection either way.
6. The daemon keeps the VibeGame's key and name in `vibegames.json` in its state directory, and connects again as below.

A VibeGame keeps up to four paired computers. "Forget computers" on the VibeGame forgets them all; "Forget" in the App forgets one VibeGame on that computer and sends `{"version":1,"event":"vibegame.unpair"}` if it is connected.

**What this does not stop.** An attacker on the same network who is in the middle at the moment of pairing could pass the code along and pair with both ends. The code is checked once, online, with three tries, so guessing does not work. A VibeGame only shows status and asks to open a window, so we accept this for now. Approving an Agent's request from the VibeGame (not in this version) needs a stronger pairing first.

## The connection

After pairing, the computer connects with **`Noise_IK_25519_ChaChaPoly_SHA256`**, prologue `vibebuddy-vibegame-v1`, computer as initiator, knowing the VibeGame's key. Framing is the box's: one line per handshake message, then one Noise transport message per line, each the ciphertext of one VibeBuddy Protocol line, as unpadded base64url (`protocol.md`, "Local network link").

- The VibeGame sends `{"version":1,"event":"link.noise","vibegame":"<VibeGame key>"}` in the clear first, as the box does with `box`.
- A computer that is not paired gets `link.error` (`auth`) in the clear, and the VibeGame closes.
- The VibeGame talks to one computer at a time. A second paired computer gets a sealed `link.busy` with `reason` `linked` and `owner`, and is closed. The first one keeps it until it goes away; a computer that has been silent for 15 s is gone. There is no takeover: the VibeGame is not shared the way the box is.
- After the handshake the VibeGame sends `{"version":1,"event":"vibegame.hello","build":"…","name":"…","screen":[720,480]}`.
- The daemon connects to every paired VibeGame it can reach, at the same time as it links to a box.

## What the computer sends

The same lines it sends the box, built from the same snapshot (`activity.rs`, `occasions.rs`), with these differences:

- **State events** (`task.start`, `agent.input_required`, `task.done`, `task.error`, `agent.blocked`, `agent.idle`) with `tasks`, `tasks_total`, `stats`, `announcement`, `announcement_id`, `occasion` and `suppress_audio`, exactly as for the box.
- **`buddy.say`** for the daily greeting, as for the box.
- **`device.heartbeat`** every 5 s, with `build`, `hour` and `day`.
- **`quiet`**: every state event and `buddy.say` carries `"quiet":true` while a box is linked to this computer, and `"quiet":false` while none is. A quiet VibeGame shows everything and plays nothing. The daemon decides, because only it knows whether a box is linked (dragon, 2026-11-11: when both are there, the box speaks).
- **The occasion is shared.** The daemon picks one occasion per announcement and sends it to the box and the VibeGame alike, so "first done of the day" is counted once.
- **Not sent**: anything that looks after a box (`device.*` except the heartbeat, `xfer.*`, Character packs, Wi-Fi, Relay, screens, decorations) and voice commands (`voice.*`). The VibeGame ignores any event it does not know.

## What the VibeGame sends

- `{"version":1,"event":"button","button":"K2","action":"press"}` when A is pressed on the duty screen. The daemon does what it does for the box's K2.
- `{"version":1,"event":"focus","task":"<id>"}` when a card is picked with the D-pad and A. The daemon does what it does for a tapped card.
- Nothing else in this version.

## On the VibeGame

- **Screens**: not paired (the code), looking for a computer, and on duty (state, cards, stats). It draws its own screens for 720×480; it does not copy the box's code. The look follows the robot screens. 
- **Sound**: the robot's lines for `input_required`, `done` and `failed`, packed into the app as WAV, at most 3 s each. Not when `quiet` is true or the VibeGame is muted. Other Characters come later.
- **Pomodoro**: runs in the app, as on the box: START starts, pauses and goes on; holding START gives up. SELECT switches between on duty and Pomodoro.
- **Lid**: closing the lid stops the app (QingOS ADR 0030). When it opens, the app waits for the computer to connect again and shows the state it sends. A VibeGame does not remind anyone while it is closed (dragon, 2026-11-11).
- **No link**: after 15 s with no line from the computer, the buddy closes its eyes and the screen turns gray, as on the box.

## The daemon's API

- `GET /v1/status` gets `vibegames`: one item per VibeGame found or paired, with `key`, `name`, `paired`, `connected` and `address`.
- `POST /v1/vibegames/pair` with `{"key","code"}`: pairs. Answers 200, or 400 with `{"error":"code"}`, `{"error":"full"}` or `{"error":"unreachable"}`.
- `POST /v1/vibegames/forget` with `{"key"}`: forgets one VibeGame.

## The apps

Mac and Linux, in the same PR (AGENTS.md):

- The Device page gets a **VibeGames** part: VibeGames found and not paired, each with **Pair…**, which asks for the six digits; paired VibeGames, with connected or not and **Forget**.
- zh-Hans strings on both sides: 掌机、配对…、输入掌机上显示的六位数、忘掉这台掌机.

## Testing without the VibeGame

- `tools/simulate-vibegame.py` (to write) answers on port 7342 as a VibeGame, prints what it gets, and sends A and `focus` from the keyboard. With `VIBEBUDDY_VIBEGAME_ADDR=127.0.0.1:7342` the daemon talks to it.
- QEMU's user network does not pass multicast, so mDNS is checked on the real handheld only. In QEMU, forward port 7342 and use `VIBEBUDDY_VIBEGAME_ADDR`.
