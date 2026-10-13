---
status: proposed
---

# Firmware and Character packs travel over any link in one resumable transfer

A box off the cable can only get new firmware or a new Character over USB today. That matters most for the Relay's certificate roots: the box checks the Relay against a fixed set compiled into the firmware (`firmware-rs/device/certs/relay-roots.pem`), so a change of root at Cloudflare that the set doesn't cover cuts every box off the Relay. An update over the air turns a new root set into a routine update, but only if it reaches boxes **before** Cloudflare changes: once the Relay is unreachable, it can't carry the fix, and only the local network or USB can. The root set therefore also has to be kept ahead of Cloudflare, starting with Let's Encrypt's ISRG Root YE and YR.

The existing Character pack write (`voice.*`, `docs/protocol.md`) is stop and wait, 672 bytes a chunk, so it is limited by the 1024-byte line. Through the Relay, at 130–250 ms a round trip, a 2 MB firmware image would take about seven minutes.

We decided that firmware images and Character packs travel the same way: a chunked, windowed, resumable transfer made of VibeBuddy Protocol lines, so it works identically over USB, the local network and the Relay, inside the link's Noise session off the cable (ADR-0015).

## Decision

**Who may send**

- Off the cable, a box accepts a transfer only inside a Noise session, so only from a paired computer that holds the link (ADR-0011, ADR-0015). Over USB it accepts one from whoever is on the cable, as it does today; the cable could flash anything through the ROM anyway.
- The box doesn't check who built an image or whether it is newer. The App offers only released firmware, verified against the signed update manifest, and never a downgrade (ADR-0010); "Flash from file…" sends whatever the user picks. A release key that the box checks is deferred (ADR-0015, Consequences).
- No confirmation on the box. The user confirms in the App (ADR-0010); a box updated through the Relay usually has nobody beside it.

**The transfer**

```json
{"version":1,"event":"xfer.begin","kind":"firmware","size":1843200,"sha256":"<hex>"}
{"version":1,"event":"xfer.chunk","offset":0,"crc":305419896,"data":"<base64, at most 3072 bytes>"}
{"version":1,"event":"xfer.end"}
{"version":1,"event":"xfer.cancel"}
```

```json
{"version":1,"event":"xfer.ready","offset":0}
{"version":1,"event":"xfer.ack","offset":12288}
{"version":1,"event":"xfer.done"}
{"version":1,"event":"xfer.error","code":"sha256","offset":0}
```

- `kind` is `firmware` or `character`. `size` and `sha256` are the whole image's. The box answers `xfer.ready` with the offset to start from, then acknowledges chunks with the next offset it expects.
- **Window.** The computer keeps at most 4 chunks unacknowledged. 3072 bytes of data are a line of about 4.2 KB, which fits the 4608-byte line limit inside a session (ADR-0015) and the Relay's 8 KiB once encrypted. The box takes only the chunk at the offset it expects and acknowledges anything else with that offset again: every way of carrying the link delivers in order, so a chunk only goes missing when the link drops. At 4 lines in flight and 130–250 ms a round trip this is about 50–90 KB/s, under the Relay's 50 messages a second, so 2 MB in under a minute; erasing flash ahead of the writes is the other limit.
- **`crc`** is the CRC32 of the chunk's raw bytes, as in `voice.chunk`. Inside a session it is redundant, but the BOX's UART bridge scrambles bytes without changing their length, and USB has no session.
- **Resuming.** The box keeps, in RAM only, the kind, size, sha256 and the offset written so far. An `xfer.begin` with the same three resumes at that offset, even over another way of carrying the link (a box that drops off the Relay and gets plugged in carries on). A different image starts over. A restart of the box starts over: two megabytes take a minute, and persisting progress isn't worth its complexity.
- **Errors.** `xfer.error` codes: `busy` (another transfer is running, or a line is being written), `too_big`, `crc`, `sha256`, `bad_image`, `flash`, `cancelled`, `sequence`. Any error ends the transfer and clears what the box kept.
- **Lines** grow from 1024 bytes to 4608 everywhere (USB too), in buffers that are static, not on the 16 KiB heap. Other events keep their own limits.
- **Capability.** Firmware that can take transfers answers `device.hello` with `TRANSFER 1`. For other firmware the daemon keeps using `voice.*` for Character packs, and USB flashing for firmware.

**Firmware**

- The box writes the OTA slot it is not running from (`ota_0` at 0x10000 or `ota_1` at 0x610000, 4 MB each), erasing 64 KB blocks ahead of the writes so `xfer.begin` answers quickly. Before writing it checks the image fits the slot and its header is an ESP32-S3 app image (magic 0xE9, chip ID 9), and answers `bad_image` otherwise.
- On `xfer.end` it reads the slot back, checks the length and SHA-256, writes the next `otadata` entry selecting that slot in state NEW, answers `xfer.done`, and restarts a second later. It never touches `otadata` before the image is whole and checked, so a transfer that fails at any point leaves the box running what it ran.
- Rollback is ADR-0013's: the bootloader runs the new image once as PENDING_VERIFY, and unless it confirms itself (`firmware-rs/core/src/ota.rs`) it goes back to the old slot. A box that finds a newer `otadata` entry ABORTED at boot prints `FIRMWARE ROLLED BACK`. The daemon decides success the same way either way: after the box reconnects, `FIRMWARE VERSION` must be the new one.
- During a firmware transfer the screen shows `UPDATING` with a percentage over whatever is underneath; events keep updating the state below it, announcements are silent, K0 and K1 do nothing, and a long K2 cancels.

**Character packs**

- The same transfer with `kind` `character`. The box checks the magic (`VBCP`) in the first chunk, keeps the header in RAM and writes it last, after the read-back check passes, then switches Character, exactly as `voice.*` does. Announcements during the write use the built-in voice.
- `voice.*` stays for firmware without `TRANSFER 1` and is removed once no supported firmware needs it.

**The App, on the Mac and on Linux alike**

- The Device tab's Update button and "Flash from file…" work whenever the box is linked, whichever way, and say how (over USB, over Wi-Fi, through the Relay). Progress shows a bar, the time left, and Cancel. Switching Character works the same way, off the cable too.
- On USB, firmware that has `TRANSFER 1` and is running is updated over the transfer, which has rollback; the ROM flasher (`daemon/src/rom_flasher.rs`) stays for a box that doesn't answer, firmware without `TRANSFER 1`, other firmware, and the one-time partition layout migration of ADR-0013.
- Firmware without `TRANSFER 1` off the cable: the App says to connect the cable to update.
- Success is the new `FIRMWARE VERSION` after the box comes back; if it comes back with the old one, the App says the update didn't take and the box is back on that version.

## Considered options

- **The box downloads the image itself** over HTTPS from the update manifest's URL. No Relay limits, but the box would need HTTP downloads and resuming of its own, it would bypass the App's confirmation, and when the roots are the problem it couldn't reach a host on Cloudflare either.
- **Larger messages or binary frames on the Relay.** Faster, but a protocol change on the closed service for something the existing limits already carry in under a minute.
- **Keep stop and wait, bigger chunks.** One chunk per round trip still costs minutes through the Relay.
- **Persist transfer progress in flash**, so a restart resumes too. Restarts mid-transfer are rare, and a whole transfer takes about a minute.
- **A separate, signed update channel for the certificate roots** (a small blob in the `link` partition). Another signing path to keep, when the roots can simply ship with the firmware now that firmware travels over the air.
- **A version check on the box** that refuses older images. Without a release key, any paired computer can send any image anyway, so it would only stop the App, which never offers a downgrade.

## Consequences

- The release that brings this needs one USB flash: firmware 0.5.0 can't take a transfer. From then on the Relay's roots are a routine firmware update, as long as new roots ship before Cloudflare uses them.
- Closes "Resuming large writes after a drop" in `docs/relay-protocol.md`, "Not decided yet". `docs/protocol.md` gets the `xfer.*` events and the new line limit; `docs/app.md` the App flow; `voice.*` is marked as kept for older firmware.
- "Flashing needs the cable" in `docs/protocol.md` is no longer true for firmware that has `TRANSFER 1`.
