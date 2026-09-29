# Implementation References and Their Limits

This document records external implementations worth learning from. The reference projects are not treated as a source of hardware facts for Vibe Buddy.

## `second-state/echokit_box`

Version checked: [`4484efca885c2ffd01ffb1acdbb5817421583bd8`](https://github.com/second-state/echokit_box/tree/4484efca885c2ffd01ffb1acdbb5817421583bd8)

Worth borrowing:

- The Rust firmware uses `esp-idf-svc` and pulls in C components through `esp-idf-sys`; this shows that "Rust on top + vendor/ESP-IDF C drivers" is a workable structure on the ESP32-S3. [Cargo.toml](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/Cargo.toml)
- The project puts board-level differences in a board module/feature, which is worth referring to if a second piece of hardware ever actually appears; Vibe Buddy's first board doesn't copy its multi-board abstraction in advance.
- The repository pins `ESP_IDF_VERSION = "v5.4.1"` and uses `xtensa-esp32s3-espidf` and `espflash`. [`.cargo/config.toml`](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/.cargo/config.toml)
- The README records that the EchoKit device enumerates as a JTAG serial port through the port labeled OTG/SLAVE, and gives a `/dev/cu.usbmodem...` example; this can serve as one troubleshooting hypothesis once the Vibe Buddy board is plugged in, but not as a conclusion about the target board. [README](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/README.md)

Not adopted directly:

- EchoKit's protocol is designed for Wi-Fi/WebSocket audio sessions, with server events in MessagePack and some device commands in JSON; Vibe Buddy v1 is local USB serial NDJSON, with different goals and framing. [`src/protocol.rs`](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/src/protocol.rs)
- The GPIO, ES8311, XL9555, 320×240 LCD and other parameters in `atom_box.rs` and `components/hal_driver` belong only to the corresponding EchoKit board. They must not be copied into Vibe Buddy unless the exact PCB/schematic proves they match.
- The repository is licensed under GPL-3.0. Until Vibe Buddy's license compatibility strategy is settled, we borrow only ideas and don't copy implementation code.

## `second-state/echokit_server`

Version checked: [`d1d976596f122976095b7da4df3e946baf152b96`](https://github.com/second-state/echokit_server/tree/d1d976596f122976095b7da4df3e946baf152b96)

Worth borrowing:

- The server uses Rust, Tokio, Axum, Serde and the tracing/logging ecosystem, which matches the direction of the candidate stack for `vibebuddyd`. [Cargo.toml](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/Cargo.toml)
- WebSocket I/O uses a separate message-handling loop and channels to separate the transport from the business pipeline; this division of responsibility is worth referring to when designing the serial send/receive/reconnect tasks in Stage 2. [`src/services/ws.rs`](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/src/services/ws.rs)

Not adopted directly:

- EchoKit Server is an ASR → LLM → TTS voice platform, far larger in scope than a local state daemon. Vibe Buddy doesn't bring in its AI providers, VAD, audio streaming, MCP or configuration system.
- Its network protocol, retries and audio chunking strategy can't replace the Vibe Buddy Protocol's versioning, line-by-line framing, input limits and unknown-event rules.
- This repository is also GPL-3.0; at this stage we don't copy its code.

## What this means for Vibe Buddy in practice

1. Keep the Rust + Tokio/Axum direction on the Mac side, but don't create dependencies or code until Stage 2.
2. The firmware still evaluates ESP-IDF C first; whether to adopt Rust firmware must be based on how reusable the official BSP is, build complexity and the Stage 1 minimal link, not chosen automatically because the reference repositories use Rust.
3. Once the board is plugged in, watch `/dev/cu.usbmodem*`, USB Serial/JTAG and the roles of the multiple USB ports closely, but don't presume the outcome.
4. Until the license strategy is settled, the reference repositories are used only for architecture comparison and troubleshooting leads.

## Firmware currently running on the device: `78/xiaozhi-esp32`

Version checked: [`v1.9.4 / 3ced7709c65a39494f5684e99111854a5bcbd8c7`](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)

On 2026-09-14 the hardware boot log reported the application `xiaozhi` 1.9.4, ESP-IDF v5.5 and the board `atk-dnesp32s3-box`, matching this pinned source version. This repository is therefore the primary implementation source for "the firmware currently running on the device", and can be used to understand the board configuration that currently works and to choose a compatible ESP-IDF version.

Limits: it is not ALIENTEK's vendor schematic/BSP, and it doesn't state the hardware revision of the PCB in the user's hands. The GPIO, LCD and audio parameters in its [`config.h`](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h) can only be candidates to verify, and can't on their own become Vibe Buddy's final hardware basis.
