# Bringing up the Rust firmware on a box

The firmware was rewritten in Rust on 2026-09-26 (ADR-0006) without a box at hand. Everything that could be verified on a Mac was verified then; the first hardware bring-up followed on 2026-09-28. This page records both, and is the checklist for flashing a new box.

## Verified on a Mac

- `just test-firmware`: the firmware-core tests, plus a pixel-by-pixel comparison of the Rust and C firmware screens (826 frames, including every frame of every leisure skit).
- The whole serial path: end-to-end tests on a fake board, and `tools/firmware-smoke.py` against the pty simulator (`tools/simulate-device.py`).
- The ES8311 register sequence, listed write by write against esp_codec_dev 1.6.2 in a test.
- The partition table, byte-identical to ESP-IDF's `gen_esp32part.py`.
- The build ID refreshes on every build: rebuilding a minute later with no source change moves its timestamp.

## First bring-up, 2026-09-28

Everything worked on the first flash; nothing needed a fix on the spot.

| Check | Result |
|---|---|
| Boot report (8 lines) | Matched the expected lines exactly |
| Screen: colors, orientation, animation | Correct |
| Audio: three announcements, volume change | Correct, no pops |
| Buttons K0, K1, K2 | Correct |
| Serial: 900-byte echo, screenshot | Passed on both the native USB port and the UART bridge |
| The app connects and reads the firmware build | Passed; no Mac-side change needed |
| Writing a voice pack; the box speaks in the new voice | Passed |
| Voice and volume survive a power cycle | Passed |
| Overnight on the UART bridge only | [pending] |

The review fixes applied before bring-up (see the PR) were in place, so it can't be shown which of those problems would have appeared on hardware.

## Where to look when something goes wrong

| Risk | Symptom | Where to look |
|---|---|---|
| ST7789 init or i8080 timing | `DISPLAY ERROR` on the serial line, or a lit but garbled or black screen | The command table in `firmware-rs/core/src/lcd.rs`. `device/src/lcd.rs` sends parameterless commands (SWRESET, SLPOUT, INVON, DISPON) with a dummy parameter byte, because esp-hal's i8080 driver can't disable the data phase the way ESP-IDF does. This is the one place the Rust firmware drives the panel differently from the C firmware. |
| Wrong colors or orientation | Red and blue swapped, mirrored or upside down | The MADCTL (0x36) values; pixel byte order (the framebuffer is big-endian) |
| ES8311 or I2S format | `AUDIO QUEUED` appears but nothing plays; or pitch shifts or noise | The sequence in `core/src/audio.rs`; the `TdmConfig` in `main.rs` |
| The audio stream stops and never recovers | Silence after a voice pack write | The stall detection in `device/src/audio.rs` |
| UART receive | More `invalid_json` with only the UART bridge connected | The interrupt and ring buffer in `device/src/serial.rs` |
| USB output blocking | Frozen screen after a night on the UART bridge alone (see LESSONS.md on output channels without a reader) | `write_usb` in `serial.rs` |
| Settings storage | Wrong pomodoro tally or volume after a restart | `core/src/storage.rs` |

## Intentional differences from the C firmware

- Settings (today's pomodoro tally, the volume) use a new storage format, so they reset once when the Rust firmware is first flashed.
- `voice.begin` interrupts the line currently playing before erasing the partition; the C firmware waited up to 10 seconds for it to finish.
- JSON parsing is stricter than cJSON: invalid UTF-8, `"version":1.0` and a non-string `message` get `ERROR invalid_message` (or `invalid_json`). The Mac never sends these. When `device.echo` is used to hunt corrupted bytes, a corrupted line now gets `ERROR invalid_json` instead of a CRC reply, which says the same thing.

## Steps

1. **Quit the VibeBuddy app** so it releases the serial port.
2. **Connect the native USB port** (`USB-SLAVE`, `/dev/cu.usbmodem…`) and flash:

   ```bash
   just flash /dev/cu.usbmodemXXXX
   ```

   It builds the three images, then writes the bootloader, the partition table and the app. The `voices` partition and the settings area are left alone.
3. **Read the boot report.** This step and step 4 both hold the serial port; exit with Ctrl-C when done:

   ```bash
   espflash monitor -S --chip esp32s3 --port /dev/cu.usbmodemXXXX
   ```

   The expected lines are:

   ```
   TALLY LOADED 0 0S DAY 0
   DISPLAY READY BUILD v0.2.1-… 2026-…
   VOICES <the voice that was installed>
   AUDIO READY
   AUDIO CODEC ES8311
   VOLUME 65
   BUTTONS READY
   READY vibebuddy-fw 0.1.0
   ```

   - On the first Rust flash, the tally and volume come back as defaults (0 and 65). That is expected: the storage format changed.
   - On a panic, esp-backtrace prints the backtrace on the same port and then resets the chip, like the C firmware did, so the boot report repeats.
4. **Run the smoke check** and answer its prompts about the screen and the sound:

   ```bash
   uv run --with pyserial python tools/firmware-smoke.py /dev/cu.usbmodemXXXX
   ```

5. **Buttons.**
   - K0: a short press starts a pomodoro; a long press abandons it.
   - K1: a short press switches between duty and pomodoro; a long press opens the menu, where K1 steps down, K0 changes the row and K2 closes.
   - K2: a long press toggles mute, and MUTE appears in the top left.
6. **Open the app.** The Device tab should show the same firmware build. Run a task in Claude Code or Codex and watch the task card, the face and the announcement.
7. **Change the voice once in the app.** When the write finishes, the box speaks in the new voice. Power-cycle it and check that the voice stays.
8. **Leave it on the UART bridge alone overnight**, with nothing on the USB port, and check the next morning that the screen still animates. This verifies that an output channel without a reader never blocks.

## Going back to the C firmware

```bash
just flash-c /dev/cu.usbmodemXXXX
```

The C firmware's NVS won't recognize the settings records the Rust firmware wrote, and erases them. Voice packs are not affected.

## Once the Rust firmware has proven itself

- Delete `firmware/`: the C sources, `host_tests`, `tools/*-c.sh`, `tools/test-*.sh`, `tools/compare-display.sh` and `tools/preview-display.sh`.
- Move the built-in PCM files from `firmware/main/assets` to `firmware-rs/device/assets`.
- Remove the C firmware paragraphs from the README.
