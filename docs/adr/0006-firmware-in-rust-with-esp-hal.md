---
status: accepted
---

# Firmware in Rust: esp-hal + embassy, with the logic in a host-testable firmware-core

The firmware was C on ESP-IDF, about 3,900 lines. The plan had been to write it in Rust from the start; that didn't stick at the time. On 2026-09-26 we rewrote it with esp-hal 1.x and embassy, no_std, without ESP-IDF. The code has two layers:

- `firmware-rs/core`: state machines, rendering, protocol handling, storage formats, and the ES8311 and ST7789 command sequences. None of it touches hardware. It belongs to the host workspace, and `cargo test` runs it on a Mac.
- `firmware-rs/device`: hardware glue only. It connects LCD_CAM, I2C, I2S, UART, USB Serial/JTAG and flash to core's `Board` trait, and builds separately with the Xtensa toolchain.

The `protocol` crate became no_std + alloc, so the daemon and the firmware compile the same `Event` type.

## Why esp-hal and not esp-idf-hal

The C firmware leaned on ESP-IDF very little:

- no LVGL; the screen is drawn by hand;
- `esp_codec_dev` only sent a series of ES8311 register writes;
- NVS held three integers and a volume;
- mbedtls was used only for base64.

esp-idf-hal would have been the cheapest port, but underneath it is still ESP-IDF, and the protocol would still exist twice. esp-hal's extra work is writing the ES8311 sequence and a settings store ourselves; neither is large.

## Rejected options

- **Keep C and share the protocol through a schema file**: this does nothing for the state machines and rendering, which could only be tested through makeshift C host harnesses.
- **esp-idf-hal / esp-idf-svc**: NVS and esp_codec_dev would work directly, but all we would gain is a Rust shell.

## Consequences

- **Screen equivalence is checked by comparison.** Until the C firmware is deleted, `tools/compare-display.sh` renders the same scenes with both firmwares, including every frame of every leisure skit, and compares them pixel by pixel. Run it after any change to the drawing code. The first port matched on all 826 frames.
- **Settings use a new format.** The old `nvs` partition now holds an append-only record log (`firmware-core/src/storage.rs`). The first time the Rust firmware runs, today's pomodoro tally and the volume reset to their defaults. Flashing the C firmware back makes NVS find pages it doesn't recognize and erase them.
- **The partition table and voice pack format are unchanged.** Changing firmware keeps the installed voice.
- **Build outputs come from different places.** The bootloader is the ESP-IDF second-stage bootloader bundled with espflash. The partition table is compiled by `tools/make-partition-table.py`, because espflash can't parse custom data subtypes; its output is byte-identical to ESP-IDF's `gen_esp32part.py`.
- **CI switches toolchains.** The firmware job uses esp-rs's Xtensa toolchain and no longer needs the ESP-IDF Docker image.
- **The C firmware stays for now.** `firmware/` is kept as a fallback (`just flash-c`) until the Rust firmware has proven itself on hardware for a while.
