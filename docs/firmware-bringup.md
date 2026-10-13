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

   It builds the three images, then writes the bootloader, the partition table and the app, plus a blank `otadata` so the box boots that app. The `voices` partition and the settings area are left alone.
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

## Microphone

The box has a microphone, and no firmware of ours has used it yet. `device.mic.record` is a bring-up diagnostic, like the screenshot: it records a few seconds and sends them back over the cable. It isn't a user feature, and neither app uses it.

### What we know, and from where

No ALIENTEK schematic or BSP for this board (ATK-DNESP32S3-BOX V1.1) could be found; the BOX3's wiki (a different board, with different I2S pins) doesn't count. So each fact below is either already seen on a box, or a candidate the diagnostic settles.

| Fact | Value | Source | Confidence |
|---|---|---|---|
| Microphone ADC | The ES8311's own mono ADC; no ES7210 | No chip at 0x40–0x43 in the I2C0 scans of two boxes ([`hardware.md`](hardware.md)); xiaozhi 1.9.4's `atk-dnesp32s3-box` uses `Es8311AudioCodec` for both input and output | High that there's no ES7210 on the scanned boxes; the ES8311 ADC is a candidate until a recording shows it |
| ES8311 address | 0x18 (7-bit) | Seen on boxes | Verified |
| ES7210 addresses checked | 0x40–0x43 (AD1/AD0 straps; esp_codec_dev's `es7210_address_t`) | esp_codec_dev | The diagnostic reports each |
| Microphone data into the ESP32 | GPIO47 (I2S DIN) | xiaozhi 1.9.4 `config.h`, `AUDIO_I2S_GPIO_DIN` | Candidate, from the same board's config; only ever configured as an input |
| Clocks | Shared with the speaker: BCLK GPIO21, WS GPIO13, no MCLK. The ESP32's receiver runs off its transmitter's clocks (signal loopback), as ESP-IDF's full duplex does | Speaker pins verified on boxes; xiaozhi creates TX and RX on one I2S port with the same pins and `GPIO_NUM_NC` for MCLK | High |
| Format and rate | Standard I2S, 16-bit, two slots, 24 kHz. Not 16 kHz: the ES8311 has one LRCK for ADC and DAC, so input runs at the speaker's rate | xiaozhi asserts input rate == output rate (24000) for this codec | High |
| Which slot is the microphone | Left = ADC, right = DAC reference (silent unless the box is speaking) | esp_codec_dev writes ES8311 register 0x44 = 0x58, "ADCL + DACR"; our sequence already does | Medium; the per-slot levels show it |
| ADC already enabled at boot | Yes: our ES8311 sequence is esp_codec_dev's `ESP_CODEC_DEV_WORK_MODE_BOTH`, the mode the C firmware used, so the ADC is powered and its serial output unmuted (registers 0x0A, 0x0E, 0x14, 0x15, 0x17) | `firmware/main/agent_audio.c`; `core/src/audio.rs` test | Verified in code |
| Microphone gain | Register 0x16, 0–42 dB in 6 dB steps; 30 dB by default (xiaozhi's `AUDIO_CODEC_DEFAULT_MIC_GAIN`) | esp_codec_dev `es8311_set_mic_gain` | High |
| Boxes on the NS4168 variant | No codec; xiaozhi reads a digital microphone straight from the same DIN with the same clocks | xiaozhi `ATK_NoAudioCodecDuplex` | Untested; the diagnostic still records |

### Running it

1. Quit the VibeBuddy app, then flash with `just flash /dev/cu.usbmodemXXXX` (step 2 above). Check the boot report still says `AUDIO READY` and that announcements still play: the speaker's I2S setup changed (the receiver now shares its clocks).
2. Record, making some noise near the box (talk, clap, tap the case) while it records:

   ```bash
   uv run --with pyserial python tools/mic-record.py --port /dev/cu.usbmodemXXXX --ms 3000 --out mic.wav
   ```

   `--gain-db` sets the microphone gain. The samples come back at about 11 s per recorded second, because the firmware writes the UART at 115200 baud whichever port you use.
3. Listen to `mic.wav`. The left channel should be your voice.

The box replies, in order:

```
MIC PROBE 0x18 ID 8311        ES8311 chip ID (0xFD, 0xFE); NONE if nothing answers
MIC PROBE 0x40 NONE           0x40–0x43: an ES7210 would answer here, with the bytes of 0x3D, 0x3E
...
MIC GAIN 30 DB                only with an ES8311
MIC RECORDING 3000 MS
MIC CAPTURED 3010 MS 288000 BYTES
MIC LEVEL 0 L <rms> <peak> R <rms> <peak>      one per 100 ms, raw units, full scale 32768
MIC BEGIN 24000 2 16 288000
MIC DATA <seq> <base64>        576 bytes of samples per line
MIC END 288000 <crc32>
```

`MIC UNAVAILABLE` means the board has no input; `MIC BUSY`, that a recording is still going out; `MIC ERROR STALLED`, that the DMA hadn't handed back enough samples a second after the recording should have ended; `MIC ERROR SHORT`, that the buffer filled up with fewer samples than asked for; `MIC ERROR MEMORY`, that the capture (PSRAM) or its DMA descriptors (internal RAM, taken only while recording) didn't fit.

The DMA writes the whole recording straight into one buffer in PSRAM, so nothing depends on the main loop keeping up. The buffer is half as big again as the recording: the I2S receiver signals an EOF every 8,190 bytes (esp-hal leaves RXEOF_NUM at its 12-bit maximum, 4,095 samples), and each EOF closes the DMA descriptor it lands in, so about every third descriptor holds only a few bytes. Each descriptor's length says how many it holds, and the samples stay in order. Each recording also prints two `MIC DEBUG` lines, 100 ms in (`T100`) and at the end (`END` or `TIMEOUT`), for when it fails:

```
MIC DEBUG T100 DESC 3/108 BYTES 8190 LENS 4080 4080 30 0 0 0 I2S RXCONF … TXCONF … INTRAW … RXCLKM … RXEOF … TXSTATE … DMA ININT … INFIFO … INLINK … INSTATE … INDSCR … INPERI … OUTSTATE … OUTPERI … DIN 200
```

- `DESC done/total` and `BYTES`: DMA descriptors (4,080 bytes each) handed back so far, and the sample bytes in them; at `T100` up to 9,600 (96 KB/s), counted only for descriptors already handed back. `DESC 0` means no sample reached memory.
- `LENS`: the first six descriptors' lengths, which show where the EOFs cut them.
- `I2S …`: I2S0's RX_CONF (0x20), TX_CONF (0x24), INT_RAW (0x0C), RX_CLKM_CONF (0x30), RXEOF_NUM (0x64), TX status (0x6C). RX_CONF bit 2 is RX_START and bit 3 RX_SLAVE_MOD; TX_CONF bit 2 is TX_START and bit 27 SIG_LOOPBACK (field offsets from the esp32s3 0.37 PAC).
- `DMA …`: GDMA channel 1's IN_INT_RAW, INFIFO_STATUS, IN_LINK, IN_STATE, IN_DSCR (the descriptor it is on), IN_PERI_SEL (I2S0 is 3), and OUT_STATE and OUT_PERI_SEL for the speaker's side of the same channel.
- `DIN`: level changes over 2000 quick reads of GPIO47. 0 means the data line doesn't move (or the codec sends exact silence). BCLK and WS can't be checked this way: they are outputs with their input path off, so their input register reads 0 however fast they run.

### Reading the result

| What you see | What it means |
|---|---|
| Left `LIVE`, rising when you talk; right silent | The microphone works on this box. Record the box's serial number in [`hardware.md`](hardware.md) |
| Both slots `SILENT` (all zeros) | Nothing drives GPIO47: wrong pin, or the ES8311's ADC output isn't on |
| Both slots stuck at one value, or `SATURATED` with no sound | GPIO47 is floating or isn't the codec's data line |
| Left `FLAT` with a level above zero | The ADC runs but the microphone isn't reaching it: try `--gain-db 42` and noise right at the mic hole; then suspect the analog path (MIC bias, which input the ES8311 selects in register 0x14) |
| `LIVE` on the right, not the left | The slots are the other way round from esp_codec_dev's 0x44 setting |
| Any `MIC PROBE 0x4x ID …` | This box has a chip where an ES7210 would be: it differs from the scanned boxes, so stop and look before trusting the ES8311 path |

Boxes of this board differ (some have a touchscreen, some don't), so run it on each box you care about and say which box in the result.

## Going back to the C firmware

```bash
just flash-c /dev/cu.usbmodemXXXX
```

The C firmware's NVS won't recognize the settings records the Rust firmware wrote, and erases them. Voice packs are not affected.

## Once the Rust firmware has proven itself

- Delete `firmware/`: the C sources, `host_tests`, `tools/*-c.sh`, `tools/test-*.sh`, `tools/compare-display.sh` and `tools/preview-display.sh`.
- Move the built-in PCM files from `firmware/main/assets` to `firmware-rs/device/assets`.
- Remove the C firmware paragraphs from the README.
