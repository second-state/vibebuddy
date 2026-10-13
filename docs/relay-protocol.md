# Relay protocol

How the daemon and the box talk to the Relay (ADR-0012). The Relay itself is the Worker `vibebuddy-relay` in the closed-source `vibebuddy-cloud` repo, which holds all of VibeBuddy's hosted services; this is the part both ends in this repo implement.

The Relay lives at `wss://relay.korekore.ai`; staging is `wss://relay-staging.korekore.ai`. Releases use production and every other build uses staging; the box uses whichever its computer on the cable uses (see "Which Relay"). The Relay never depends on update checks, which can be off.

## Which Relay

The daemon picks its Relay in this order:

1. `VIBEBUDDY_RELAY_URL`, for example `ws://127.0.0.1:8787` under `wrangler dev`.
2. In a release build only, the update manifest's optional `relay` field: a `wss://` address, used from the next connection on.
3. The build's default: `wss://relay.korekore.ai` for a release build (CI builds of a tag, compiled with `VIBEBUDDY_RELEASE_TAG=1`), `wss://relay-staging.korekore.ai` for everything else, which includes CI builds of main and builds from source.

The box follows the computer on its cable. Each time the daemon links over USB to a box that reports `BOX KEY`, it sends its own Relay with `device.relay.set` (docs/protocol.md, "Relay"), unless that Relay isn't a `wss://` host the box can use, such as a local `ws://` one. The box keeps it in the `link` partition and, if it changed, drops its Relay socket and connects to the new one. Firmware that has never been told uses `wss://relay.korekore.ai`. The Relay switch in the App (config `relay`) turns the daemon's own use of the Relay off; it doesn't change which Relay the box is given.

Two effects follow, both accepted while development builds stay inside the team:

- An App from before this rule (0.4.1 and older) never sends a Relay, so a box a development build moved to staging stays there under it. Those Apps have no Relay client, so nothing is lost; the first release App with this rule moves the box back on its first USB link.
- A box is on the Relay of the last computer it was plugged into. A user's box plugged once into a development build moves to staging, and the user's release App can then reach it over the local network or USB but not the Relay until it is plugged into that computer again. If development builds ever reach people outside the team, only an explicit opt-in should send staging to a box.

## Connecting

Both the box and a computer open a WebSocket to `wss://<relay>/v1/boxes/<box-id>`. `<box-id>` is the box's Ed25519 public key, 32 bytes as unpadded base64url (43 characters), so a box needs no registration. Every message either way is one JSON object with a `t` field. A box takes at most 4 sockets still authenticating from one IP (past that the upgrade gets HTTP 429) and at most 16 sockets in all; past 16 the oldest socket still authenticating is closed with `{"t":"error","code":"auth_timeout"}` and 4001 to make room. Authenticated sockets are bounded by one per key.

The Relay first sends `{"t":"challenge","nonce":"…"}`. The client answers:

```json
{"t":"hello","role":"box","key":"<public key>","sig":"<signature>","v":1}
```

`role` is `box` or `computer`, `key` the client's own public key, `v` the protocol version the client speaks (an integer; absent means 1), and `sig` its Ed25519 signature, base64url, over the UTF-8 bytes of

```text
vibebuddy-relay-v1\n<role>\n<box-id>\n<nonce>
```

A box must use the key that is its id. A computer must be in the box's paired list. The Relay replies `{"t":"welcome","v":1}` (its own protocol version; parsers ignore fields they don't know), or `{"t":"error","code":"auth"|"not_paired"}` and closes (4001, 4003). A socket without a valid hello within 10 s gets `{"t":"error","code":"auth_timeout"}` and close 4001.

One socket per key: a new connection from the same box, or the same computer, replaces the old one (close 4002). A computer's Relay link survives the swap: the box isn't told, the new socket's `status` shows it linked, and data flows without sending `link` again.

A `v` that isn't an integer gets `{"t":"error","code":"auth"}` and close 4001. A `v` older than the Relay's minimum gets `{"t":"error","code":"upgrade"}` and close 4005: the client stops retrying and says an update is needed. The minimum is 1 today. The Relay raises its version when it adds something clients may rely on, and its minimum only when it drops support for old clients.

## Limits and keepalive

- A message over 8 KiB gets `{"t":"error","code":"too_big"}` and close 1009.
- Each socket may send a burst of 100 messages, refilled at 50 a second; past that it gets `{"t":"error","code":"rate"}` and close 1008. A linked computer dropped this way counts as gone.
- Clients send WebSocket ping frames every 30 s, and treat the link as dead after 75 s with no frame from the Relay, then reconnect with backoff. Ping frames keep NATs open, but the Relay can't see them.
- The sign of life the Relay does see is the exact text `{"t":"ping"}` (no spaces, no other fields), which it answers with the exact text `{"t":"pong"}` without waking. The box sends it every 30 s. While a computer is connected to the box, a box with no message and no answered ping for 75 s is closed with 4004 `silent`, and computers get `status` with `box: false`; the box reconnects with backoff, says hello again and re-sends `paired`. With no computer connected the box isn't watched. Any other text is a normal message.

## The box

- `{"t":"paired","computers":[{"key":"…","name":"…"}]}`: the full paired list, at most 16, sent after every welcome and whenever it changes. Computers no longer in it are dropped (`unlinked`, reason `unpaired`, close 4003).
- `{"t":"linked","via":"usb"|"lan","computer":"<key>"|null}`: the box linked to a computer without the Relay (`null` for an unpaired computer over USB). Any Relay link ends.
- `{"t":"unlinked"}`: that link ended. If that computer wasn't paired (`computer: null`), the Relay frees the box at once, since such a computer can't come back through it; a paired one keeps the 10-minute grace.

When the box's socket closes for any reason (a close, an error or silence), the Relay ends a USB or LAN link the box had reported as if `unlinked` had come: a paired computer gets the 10-minute grace, an unpaired one is cleared at once, and computers get `status` with `box: false` and the owner no longer linked. The box says what it is linked to again after every welcome, so a box that reconnects while still on the cable claims it again straight away.
- `{"t":"data","line":"…"}`: one line for the computer linked through the Relay. Inside a Noise session (below) it is sealed.

The Relay sends the box `{"t":"linked","computer":"<key>","name":"…"}` when a computer links through it (also right after welcome, if one is linked), `{"t":"unlinked","reason":…}` when that link ends, and `data` from that computer. Reasons for `unlinked` to the box: `gone` (the computer's socket closed or was dropped), `released` (it sent `unlink`), `unpaired`.

## A computer

After welcome the Relay sends `{"t":"status","box":<online>,"owner":{"key","name","via","linked"}|null}`, and again whenever either changes.

- `{"t":"link"}`: link if the box is free, already ours, or its computer has been gone at least 10 minutes. Answered with `{"t":"linked"}` or `{"t":"busy","reason":"linked"|"usb","owner":"<name>"}`.
- `{"t":"takeover"}`: the user chose to take the box. Always granted unless another computer's USB cable is in (`busy`, reason `usb`).
- `{"t":"unlink"}`: give the box up.
- `{"t":"data","line":"…"}`: one line for the box, only while linked (`error`, code `not_linked`, otherwise).

A linked computer gets `{"t":"unlinked","reason":"takeover"|"moved"|"unpaired"}` when it loses the box; `moved` means it linked to the same box another way.

A computer linked through the Relay must send something at least every 75 s; the daemon's `device.heartbeat` every 5 s covers it. One that stays silent longer, such as a laptop gone to sleep, counts as gone: the grace period starts, the box gets `unlinked` with reason `gone`, and the socket is closed with 4004 `silent`. The daemon treats that as any other drop: it reconnects and asks to link again, which succeeds unless another computer linked in the meantime.

When the box reconnects to the Relay while a computer is linked through it, that computer gets `{"t":"linked"}` again. It means the box came back, perhaps after a restart: the computer starts its session over, as on a new cable connection, by sending `device.hello`.

## The session inside the Relay

The Relay carries a computer's lines but can't read or forge them (ADR-0015). Whenever the Relay links a computer to the box (`linked`, also after either reconnects), the box sends `{"version":1,"event":"link.noise","box":"<box key>"}` as `data`, and the two run the same Noise handshake as over the local network ([`protocol.md`](protocol.md), "Local network link"), each handshake message one `data` line. The box accepts a session only from the computer the Relay linked. After it every `data` line either way is one sealed line: sealing adds a 16-byte tag and base64url a third, so the box's longest line (a screenshot line under 2 KB) is under 2.8 KB, well within the 8 KiB limit. The box acts on nothing in the clear; a line that doesn't open makes it drop the session and send `link.noise` again, and makes the daemon end the link and reconnect.

The daemon starts a session whenever `link.noise` arrives and holds what it would send until the session is up, then sends `device.hello` first. Firmware before ADR-0015 never sends `link.noise`; the daemon talks to it in the clear, unless the box has spoken Noise to it before (`box-noise`, see "Local network link"), and then it waits for `link.noise` and believes nothing in the clear.

The Relay itself is unchanged: `data` was always forwarded without being read.

## The daemon

The daemon reaches the box the closest way it can: the cable, then the local network, then the Relay (`daemon/src/relay.rs`). It moves between them without a drop: `/v1/status` goes from one connection to the next with no `connected: false` in between, and `device.network` and `device.relay` say which it is. While linked through the Relay it looks for the box on the cable every 2 s and on the local network every 30 s. It sends `link` unless the user chose Take over in the App (`POST /v1/device/takeover`), which sends `takeover` once. A `busy` answer goes to `/v1/status` as `device.busy` (`owner`, `reason`), and the daemon stays on its socket: it asks again whenever `status` shows the box free, sends `takeover` there the moment the user asks, and sends `{"t":"ping"}` every 30 s meanwhile. While linked, `status` with `box: false` means the box is gone (the link reads as down) until `linked` comes again. Failed tries back off: 2 s when unreachable, 30 s on 429 or close 1008, 60 s on a refusal, doubling up to 5 minutes, and a link that ends within a minute counts as an unreachable try, so the daemon never reconnects in a tight loop; sockets the daemon gives up on purpose close with 1000. Which Relay it uses, and which it gives the box, is in "Which Relay"; `/v1/status` reports the daemon's as `device.relay_server` and the box's as `device.box_relay`; the Apps don't show them, and the box's NETWORK view names its Relay. The user can turn the Relay off in the App's settings (config `relay`, on by default); off, the daemon doesn't try it and leaves a link through it.

The simulator plays the box on the Relay with `tools/simulate-device.py --relay ws://127.0.0.1:8787`, against `wrangler dev` in vibebuddy-cloud; `npx wrangler dev --var GRACE_MS:5000` shortens the grace period for such tests (production never sets it).

## Not decided yet

Noticing a box that lost power without closing its socket: the Relay shows it online until the edge notices.
