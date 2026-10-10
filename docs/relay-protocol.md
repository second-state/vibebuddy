# Relay protocol

How the daemon and the box talk to the Relay (ADR-0012). The Relay itself is a separate, closed-source service; this is the part both ends in this repo implement.

## Connecting

Both the box and a computer open a WebSocket to `wss://<relay>/v1/boxes/<box-id>`. `<box-id>` is the box's Ed25519 public key, 32 bytes as unpadded base64url (43 characters), so a box needs no registration. Every message either way is one JSON object with a `t` field. The Relay answers WebSocket pings itself.

The Relay first sends `{"t":"challenge","nonce":"…"}`. The client answers:

```json
{"t":"hello","role":"box","key":"<public key>","sig":"<signature>"}
```

`role` is `box` or `computer`, `key` the client's own public key, and `sig` its Ed25519 signature, base64url, over the UTF-8 bytes of

```text
vibebuddy-relay-v1\n<role>\n<box-id>\n<nonce>
```

A box must use the key that is its id. A computer must be in the box's paired list. The Relay replies `{"t":"welcome"}`, or `{"t":"error","code":"auth"|"not_paired"}` and closes (4001, 4003). A second box connection replaces the first (4002).

## The box

- `{"t":"paired","computers":[{"key":"…","name":"…"}]}`: the full paired list, at most 16, sent after every welcome and whenever it changes. Computers no longer in it are dropped (`unlinked`, reason `unpaired`, close 4003).
- `{"t":"linked","via":"usb"|"lan","computer":"<key>"|null}`: the box linked to a computer without the Relay (`null` for an unpaired computer over USB). Any Relay link ends.
- `{"t":"unlinked"}`: that link ended.
- `{"t":"data","line":"…"}`: one VibeBuddy Protocol line for the computer linked through the Relay.

The Relay sends the box `{"t":"linked","computer":"<key>","name":"…"}` when a computer links through it (also right after welcome, if one is linked), `{"t":"unlinked","reason":…}` when that link ends, and `data` from that computer.

## A computer

After welcome the Relay sends `{"t":"status","box":<online>,"owner":{"key","name","via","linked"}|null}`, and again whenever either changes.

- `{"t":"link"}`: link if the box is free, already ours, or its computer has been gone at least 10 minutes. Answered with `{"t":"linked"}` or `{"t":"busy","reason":"linked"|"usb","owner":"<name>"}`.
- `{"t":"takeover"}`: the user chose to take the box. Always granted unless another computer's USB cable is in (`busy`, reason `usb`).
- `{"t":"unlink"}`: give the box up.
- `{"t":"data","line":"…"}`: one line for the box, only while linked (`error`, code `not_linked`, otherwise).

A linked computer gets `{"t":"unlinked","reason":"takeover"|"moved"|"unpaired"}` when it loses the box; `moved` means it linked to the same box another way.

## Not decided yet

How the daemon carries a Relay link next to USB and the local network, keepalive intervals, and resuming large writes after a drop.
