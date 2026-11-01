# Plan: VibeBuddy for the XGGF 64×8 display

Build an **experimental, USB-connected VibeBuddy firmware** for the XGGF PixelClock on a new branch, tentatively `xggf-64x8`. Its first milestone is a reliable, readable indicator of Agent activity. It will use the existing VibeBuddy daemon and protocol, but a purpose-built display and device runtime sized for the ESP32-C3 and its 64×8 LED matrix.

The first version will deliberately omit voice, wireless links, custom Character packs, timers and the stock clock features. Those omissions should be visible in the product documentation and, where the apps offer actions the device cannot perform, in both the Mac and Linux apps.

## Current status — 2026-11-01

The original USB-only implementation is in place and has been extended. The firmware now supports the compact renderer, bounded USB protocol, daemon state resynchronization, encrypted local Wi-Fi with mDNS discovery and persistent identity, RTC/environment readings, a bounded identify buzzer, an I2S microphone-driven sound visualizer and all three physical buttons with short/long gestures and on-panel feedback. The Mac and Linux Device pages can scan nearby networks and provision Wi-Fi over USB, then drive the device over the encrypted network link. Speech recording and transfer now work over USB and the encrypted Wi-Fi link — bounded, opt-in recordings the Device pages of both apps can drive, play and save (`VIBEBUDDY_XGGF_AUDIO=1`); see `firmware-xggf/AUDIO_PROBE_REPORT.md`. Voice commands and speech recognition, Character packs, Relay, timers and stock clock functions remain unsupported.

The physical unit passed a 15-minute rotating-state soak with no resets, stable stack headroom and panel timing, and no meaningful heap degradation. The laptop-speaker microphone test produced a clear amplitude response, and the microphone also detected the device's own identify tone. A `device.mic` command now opens a ten-second sound-reactive buddy and ten-bar level display; Agent events preempt it until another explicit microphone command. See `firmware-xggf/WORK_NOTES.md` for measurements and test commands.

The remaining first-milestone acceptance work is a measured panel waveform, power observations, Mac app validation, and a final stock-restore/prototype-reinstall cycle. A future product option is automatic ambient visualizer activation while offline, with an explicit setting and unchanged Agent priority. A measured waveform requires suitable external test equipment; it cannot be established from the working display alone.

## What the investigation establishes

- The examined XGGF unit identifies itself as an **ESP32-C3 with 4 MB flash** and exposes Espressif USB Serial/JTAG as `303A:1001`. Its flash and security configuration were read successfully on that unit. This does not establish that every production unit has identical hardware or eFuses. See the [security probe](/mnt/dell-ext/Workspace/XGGF/ESP32_SECURITY_PROBE_REPORT.md:1).
- The reverse engineering describes a **64×8, 512-pixel WS2812 panel** driven by SPI2, with GPIO 21 as data, RGB framebuffer values encoded as GRB on the wire, and a column-major physical layout with Y inversion. These are strong starting hypotheses from the stock image; the coordinate mapping and waveform still need testing on the target unit. See the [hardware guide](/mnt/dell-ext/Workspace/XGGF/DEV_GUIDE.md:150).
- [PixelClock_template](/mnt/dell-ext/Workspace/XGGF/PixelClock_template/README.md) is useful reference code, not a proven drop-in foundation. Its [review](/mnt/dell-ext/Workspace/XGGF/PixelClock_template/Review-2026-07-04.md) says the review was static, identifies a raw-frame layout mismatch, and leaves the SPI clock-pin choice unresolved. The template uses 3.2 MHz and a longer latch interval; the reconstructed stock driver used approximately 4 MHz. Both choices require physical verification.
- VibeBuddy’s current device firmware is built for an **ESP32-S3 box**, with a substantially larger display and hardware-specific audio, networking and storage. Hardware-independent logic lives in [firmware-rs/core](/mnt/dell-ext/Workspace/vibebuddy-cardputer/firmware-rs/core/Cargo.toml), but bringing the entire core onto the C3 would also pull in features and dependencies this first version does not need.
- The daemon already sends versioned newline-delimited events and recognizes firmware through `DISPLAY READY BUILD`. It also deliberately avoids sending commands to foreign firmware. That discovery behavior must remain intact; see [serial transport](/mnt/dell-ext/Workspace/vibebuddy-cardputer/daemon/src/serial_transport.rs:49).

## Target behavior

Use a fixed, compact layout with an immediately recognizable buddy and a short status label. Design and preview actual 64×8 pixel art before committing to exact widths.

| State | Display behavior |
|---|---|
| Link unavailable | Dim gray buddy with closed eyes and a short offline indicator |
| Idle | Calm buddy and `READY` |
| Working | Small looping animation and `WORKING` |
| Input required | Distinct amber animation and `INPUT` |
| Done | Brief green celebration and `DONE`, then return according to existing VibeBuddy state semantics |
| Failed or blocked | Distinct red expression and `FAILED` |
| Identify | Brief, bounded visual animation, then restore the current state |

Use shape and animation as well as color to distinguish important states. Keep brightness conservative and capped; never display unbounded full-white output. Do not squeeze existing box artwork or long task titles onto eight rows. If later testing shows a short scrolling title is useful, treat it as a separate enhancement.

The daemon remains responsible for Agent aggregation and priority. The display renders the state it receives; it must not invent a separate priority scheme.

## Implementation sequence

### 1. Create the branch and establish a baseline

- Start `xggf-64x8` from the intended current project base after checking the worktree and branch state.
- Record the existing firmware, daemon and host test commands, and run the relevant baseline checks before changing shared code.
- Inventory the current daemon’s initial connection and reconnect behavior: establish exactly when it sends the current aggregate state, how it handles a restarted device, and the existing timeout and done-to-idle rules. Reuse those semantics rather than guessing from individual events.
- Document the prototype’s supported and omitted features before making app changes.

### 2. Verify the specific device and make restoration possible

Before writing firmware to hardware:

- Identify the unit and record its chip, flash size, USB identity, boot behavior, security configuration and partition table.
- Read and hash a **complete, device-specific flash backup**, including its NVS and stock data. Keep the backup outside version control because it can contain device settings and credentials.
- Write down and test a restoration procedure appropriate to that exact unit. Preserve the backup and record the addresses used; do not assume another unit’s dump is interchangeable.
- Record power-supply capacity and begin with a very low brightness limit. Check for resets and excessive heating during later tests.

The examined unit’s readable flash is encouraging, but neither its eFuse results nor its recovery behavior should be assumed for a different unit.

### 3. Bring up only the display

Create a small, separate ESP-IDF ESP32-C3 firmware target, using the sibling template as a reference for pin assignments and encoding. Keep the existing ESP32-S3 firmware and its release artifacts separate.

- Start with USB logging and a minimal panel driver. Use a bounded framebuffer and transfer buffer; document their RAM cost and SPI DMA requirements.
- Use the recovered nibble encoding and GRB output as the starting point. Verify the SPI frequency, latch timing and whether a clock pin must be routed on the actual board. Do not drive GPIO 7 until its board connection has been checked.
- Establish one documented coordinate convention: logical top-left, row-major pixels in renderer tests, with physical mapping isolated in the panel driver.
- Test a dim single-pixel walk, corners, horizontal and vertical lines, RGB channels and an asymmetric pattern. This settles origin, direction, column ordering, color order and any serpentine wiring without relying on the template’s inconsistent raw-frame helpers.
- Add a host-side encoder test with known byte sequences and bounds checks. Measure heap, stack and transfer time on the C3.

**Gate:** proceed only after the physical patterns match the documented mapping and remain stable across repeated refreshes and reboot.

### 4. Implement the compact VibeBuddy renderer

Keep the renderer and state transitions hardware-independent so they can run in host tests and a preview tool.

- Draw a small built-in buddy, compact uppercase glyphs and a few bounded animation frames directly for 64×8.
- Implement clipping and deterministic frame timing; avoid allocations on each frame.
- Preview every state, transitions, identify animation and disconnected state at native pixel resolution and enlarged. Check legibility on the actual LEDs at a normal viewing distance.
- Match the established VibeBuddy event-to-state behavior in [firmware.rs](/mnt/dell-ext/Workspace/vibebuddy-cardputer/firmware-rs/core/src/firmware.rs:1739), including the done timeout, as confirmed in step 1.
- Make heartbeat loss close the buddy’s eyes and dim it. Verify reconnection restores the **current** aggregate state, including when the daemon or device restarts; add a daemon resynchronization path if the existing one does not guarantee this.

### 5. Integrate the existing USB protocol

Implement the smallest compatible protocol subset in the C3 runtime:

- Emit the expected build marker and a distinct board identity, for example `BOARD xggf-64x8`, at boot and in response to `device.hello`.
- Accept `device.heartbeat`, `device.identify`, and the existing Agent state events: `task.start`, `agent.idle`, `agent.input_required`, `task.done`, `task.error`, and `agent.blocked`.
- Parse bounded NDJSON incrementally, including split and combined reads. Reject malformed, unsupported-version and oversized messages safely; ignore or explicitly report unsupported commands without claiming they succeeded.
- Keep diagnostics distinguishable from protocol messages and compatible with the daemon’s foreign-firmware detection. Verify boot logs do not cause our own firmware to be classified as foreign.
- Preserve the daemon’s one-box selection and foreign-firmware protections. Since XGGF shares the generic Espressif USB ID with other boards, identify the device from its firmware and remembered serial number, not VID/PID alone.

Add end-to-end tests using a simulated serial peer: startup, hello, real daemon events, heartbeat loss, reconnect, reset, malformed input, unsupported events and a foreign ESP-IDF device that must receive no unintended writes.

### 6. Make device capabilities and update safety explicit

A display-only device cannot honestly present itself as the existing box. Introduce a board/capability representation sufficient for the daemon and both apps to distinguish it.

- Ensure the existing box firmware is **never offered or flashed** to XGGF, including onboarding, manual install, update and recovery paths. Existing update gating currently distinguishes the released board in [daemon status](/mnt/dell-ext/Workspace/vibebuddy-cardputer/daemon/src/status.rs:15); audit every flashing entry point, not just update offers.
- Show the same device identity and supported features in the Mac and Linux apps. Disable or clearly explain unavailable actions such as voice, Character installation, Wi-Fi pairing, Relay, device buttons and box-only updates.
- Keep initial installation explicitly board-specific. Do not advertise automatic updates for XGGF until there is a separately identified, board-compatible release and recovery mechanism.
- Add Chinese translations on both platforms and run `tools/check-localization.py`. Update both READMEs section for section if either changes, plus architecture and device documentation.

### 7. Validate on hardware and define completion

Run the host tests and both app checks, build the C3 firmware, and then test the actual device through the real daemon on both supported desktop platforms where hardware access is available.

Acceptance requires:

1. The stock image has a verified backup and a documented restoration path.
2. Panel mapping and colors match the host preview; brightness stays within the configured cap.
3. Every state in the table appears from real daemon events, including competing Agent activity and the correct aggregated priority.
4. Disconnect, daemon restart, device reboot and reconnect lead to the correct current state without a stale working indicator.
5. Malformed and unsupported input cannot crash the firmware or falsely report success.
6. Mac and Linux identify the device consistently, explain unavailable features and never offer incompatible box firmware.
7. The device runs for an extended soak with no unexplained resets, flicker or heap degradation; record refresh timing, memory headroom and power observations.
8. Restore the stock firmware from the unit’s backup and verify it boots, then reinstall the prototype and verify it boots again.

## Decisions deferred until evidence exists

- **SPI timing and GPIO 7:** choose from measurements and physical tests, not from the conflicting reconstructed and template claims.
- **Reuse of Rust core:** extract a small shared state component only if it can be reused without dragging the S3 display, audio and network dependencies onto the C3. Otherwise retain the existing protocol contract and test the compact implementation against shared scenarios.
- **Microphone, buzzer, sensors, Wi-Fi and stock clock functions:** the reverse engineering suggests these peripherals exist, but they are outside the first milestone. Add them only after the display-only product works and their wiring and usefulness are verified.
- **Licensing and asset provenance:** check the sibling template’s license and the provenance of recovered stock fonts or artwork before copying anything. Prefer original minimal VibeBuddy artwork and independently implemented protocol code.
