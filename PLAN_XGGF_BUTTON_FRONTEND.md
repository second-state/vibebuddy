# Plan: XGGF Three-Button Frontend Expansion

**Date:** 2026-11-01
**Status:** Implementation plan only; no firmware, daemon, or app behavior is changed by this document.
**Evidence base:** `docs/investigation-report-xggf-three-buttons.md` and `docs/investigation-report-xggf-button-expansion.md`.

## 1. Objective and outcome

Make the XGGF's three buttons a complete, discoverable frontend for capabilities that are already reliable on the device and daemon:

- preserve the verified left/middle/right mapping and all current shortcuts;
- add a compact menu behind K1 long;
- expose read-only link and pairing status;
- let K0 stop an active speech recording safely;
- add bounded brightness control after the core menu works;
- correct Mac and Linux capability text and provide matching button guidance.

The result should reuse existing VibeBuddy behavior wherever possible, avoid weakening audio or pairing security, and remain testable on the host before each physical flash.

## 2. Current assets to reuse

- Physical mapping: **K0 = left/GPIO8, K1 = middle/GPIO9, K2 = right/GPIO2**, all active-low.
- Debounce, one-second long press, and `DOWN`/`LONG` panel feedback: `firmware-xggf/main/buttons.c`, `main.c`, and `status.c`.
- Agent, clock, and environment pages: `firmware-xggf/main/status.c`.
- Microphone visualizer and bounded speech recording: `microphone.c` and `audio.c`.
- Existing daemon K2 source routing: `daemon/src/main.rs` and `activity.rs`.
- Wi-Fi, pairing, IP, SSID, RSSI, and connection diagnostics: `firmware-xggf/main/wifi_link.c`.
- Existing menu convention: K1 long opens, K1 moves, K0 selects, K2 closes (`docs/device-menu.md`).
- Existing physical test harness: `firmware-xggf/tests/buttons.py`, `physical_serial.py`, `soak.py`, and `wifi_smoke.py`.

## 3. Scope and non-goals

### In scope

1. Menu navigation and on-panel hints.
2. Read-only LINK view with transport, Wi-Fi, IP, and paired-count information.
3. Stop-active-recording through a device-to-daemon request.
4. Bounded brightness selection and persistence.
5. Mac/Linux XGGF button guidance and zh-Hans translations.
6. Host, daemon, physical USB, encrypted Wi-Fi, and soak verification appropriate to each change.

### Explicit non-goals for this stage

- No button-based recording start or voice push-to-talk.
- No speech-recognition or voice-reply implementation.
- No unpair, forget-all, new-identity, or forget-network button action.
- No Pomodoro, Leisure, Character, Relay, or stock-clock feature port.
- No change to the GPIO mapping or the existing maximum channel cap of 8.
- No redesign of the audio transport while its separate reliability plan is unresolved.

## 4. Target interaction contract

### Normal display

| Gesture | Behavior |
|---|---|
| K0 short | If speech recording is active, request recording stop; otherwise toggle the microphone visualizer. |
| K0 long | Identify animation and tone, respecting mute. |
| K1 short | Cycle Agent → clock → environment. |
| K1 long | Open the menu on its first row. |
| K2 short | Emit the existing `button` event so the daemon opens the current Agent source. |
| K2 long | Toggle buzzer mute. |

The panel continues to show `K0/K1/K2 DOWN` while held and `LONG` when the long threshold fires. Agent events retain priority and return the display to Agent status.

### Menu

| Gesture | Behavior |
|---|---|
| K1 short | Select the next row, wrapping at the end. |
| K0 short | Open or activate the selected row. |
| K2 short | Close the menu; inside LINK, return to the menu first. |
| K2 long | Toggle buzzer mute without changing the global shortcut. |
| Other long presses | No menu action. |
| 30 seconds without a key | Close the menu. |

Initial rows, in order:

1. **MIC** — start/stop the existing visualizer; show its current state.
2. **LINK** — open the read-only link pages.
3. **BRIGHTNESS** — cycle the supported levels.
4. **IDENTIFY** — run the existing identify behavior.
5. **MUTE** — toggle and show the current buzzer state.
6. **CLOSE** — return to the previous normal page.

New Agent activity closes the menu. Recording indication has priority over ordinary menu content; a button press must not hide an active `REC` indication longer than necessary to show its own `DOWN`/`LONG` feedback.

### LINK pages

Show one short value at a time, using K1 to advance and K0 or K2 to return:

1. **LINK** — `USB`, `LAN`, or `OFFLINE`.
2. **WIFI** — configured SSID, or `NOT SET`; show joining/failure state where applicable.
3. **IP** — assigned address, or `NO IP`.
4. **PAIRS** — paired-computer count.

Never render passwords, private keys, full pairing keys, or audio data.

## 5. Implementation phases

### Phase 0: Baseline and change isolation

1. Inspect `git status` and preserve all existing work; do not reset or overwrite unrelated changes.
2. Re-run the current XGGF host tests and C3 build before editing:
   - panel, status, and buttons C tests from `firmware-xggf/README.md`;
   - `idf.py build` with the documented `IDF_TOOLS_PATH` and ESP-IDF environment.
3. Confirm the recovery backup still exists in the private, ignored probe directory. Do not read or print its contents.
4. Keep all physical flashing and testing under the repository-root `DEVICE.lock`.

**Exit gate:** the unmodified baseline builds and tests pass, and recovery material is present.

### Phase 1: Menu state and rendering

**Primary files:**

- `firmware-xggf/main/status.h`
- `firmware-xggf/main/status.c`
- `firmware-xggf/main/main.c`
- `firmware-xggf/main/buttons.c` only if event routing needs adjustment
- `firmware-xggf/tests/status_test.c`
- `firmware-xggf/tests/buttons_test.c`
- `firmware-xggf/tests/buttons.py`

**Implementation:**

1. Add a hardware-independent menu state alongside `status_view`, rather than embedding GPIO logic in rendering:
   - open/closed state;
   - selected row;
   - optional LINK subview and page;
   - last-key timestamp for the 30-second timeout.
2. Keep menu decisions in pure C functions so host tests can drive them without ESP-IDF.
3. Route K1 long to menu-open instead of `status_home`.
4. Preserve all normal K0/K2 shortcuts and K2 long mute.
5. Give Agent events priority over the menu and close it on their arrival.
6. Add compact row labels and current-state values. Extend the 3×5 glyph table only for characters actually required; avoid shrinking the font.
7. Update the guided test to check menu events and visible labels rather than relying only on serial output.

**Host tests:**

- open on K1 long, next/wrap, select, close, and timeout;
- K2 short closes instead of emitting the source event while the menu is open;
- K2 long still toggles mute;
- Agent events close the menu and restore Agent status;
- MIC, IDENTIFY, MUTE, and CLOSE actions preserve existing behavior;
- button feedback does not permanently hide recording or Agent priority states.

**Exit gate:** all menu state transitions and rendering tests pass on the host; the firmware builds and current physical shortcuts still work after flashing.

### Phase 2: Read-only LINK status

**Primary files:**

- `firmware-xggf/main/wifi_link.h`
- `firmware-xggf/main/wifi_link.c`
- `firmware-xggf/main/status.h`
- `firmware-xggf/main/status.c`
- `firmware-xggf/main/main.c`
- `firmware-xggf/tests/status_test.c`

**Implementation:**

1. Define a bounded `wifi_link_status` value containing only:
   - configured SSID;
   - Wi-Fi connection/error state;
   - IPv4 address;
   - paired-computer count;
   - whether an encrypted LAN peer is active.
2. Provide a read-only getter. Return a snapshot by value or copy fields under a short bounded critical section; do not expose pointers to mutable credential storage.
3. Track the transport of the last valid protocol line in `receive_line` (`usb == true` versus LAN) so `USB` and `LAN` are based on actual traffic, not merely the presence of a socket.
4. Populate the LINK pages from this snapshot and the existing Agent-link timeout.
5. Keep Wi-Fi provisioning and pairing mutation in their existing USB/app paths.

**Tests:**

- no network configured: `NOT SET` and `NO IP`;
- USB active, LAN active, and offline display;
- connecting, joined, failed, and disconnected Wi-Fi states;
- paired-count changes without exposing key material;
- long SSIDs are clipped safely without overrunning the 64×8 renderer.

**Physical acceptance:**

- verify LINK over direct USB;
- pair and reconnect through mDNS/encrypted Wi-Fi, then verify LINK shows LAN and the expected non-secret values;
- unplug USB and allow the normal link timeout to confirm offline behavior.

**Exit gate:** LINK is accurate on both transports, and no password or key material appears in firmware output, status, tests, or logs.

### Phase 3: Stop an active recording with K0

**Primary files:**

- `firmware-xggf/main/main.c`
- `firmware-xggf/main/audio.c` only if stop-state support is needed
- `firmware-xggf/main/status.c`
- `daemon/src/main.rs`
- `daemon/src/audio.rs`
- `docs/protocol.md`
- `firmware-xggf/tests/audio_probe.py` or a focused new physical test in the existing test suite
- daemon unit tests in the existing test modules

**Protocol addition:**

```json
{"version":1,"event":"audio.stop.request"}
```

This is device-to-daemon only. It is not an authorization to start, retrieve, or delete audio.

**Firmware behavior:**

1. When `audio_active()` is true, K0 short emits `audio.stop.request` over USB and encrypted Wi-Fi instead of printing only `MIC BUSY`.
2. Mark the request pending and keep `REC` visible until the existing `device.audio.stop` arrives.
3. Handle duplicate requests idempotently.
4. If no stop response arrives within two seconds, stop locally with the existing bounded `audio_stop()` path and report a diagnostic timeout. This favors privacy and the existing 60-second bound; the daemon may discard an unvalidated early recording rather than pretending it succeeded.
5. Preserve `AUDIO END`, recording cleanup, and sample-rate restoration.

**Daemon behavior:**

1. Handle the event in the existing device-event path.
2. Only when an audio session is active, call `AudioIngestor::stop()` and send `device.audio.stop`.
3. Ignore idle or duplicate requests without starting a new session.
4. Do not change `VIBEBUDDY_AUDIO_PROBE_TOKEN` requirements for HTTP start, stop, delete, WAV, or stats endpoints.
5. Preserve disconnect errors and never log PCM content.

**Daemon tests:**

- active recording becomes allowed to finish short and produces a valid WAV;
- idle and duplicate requests do not create a session;
- stop requests cannot retrieve, delete, or start audio;
- early `AUDIO END` after the request validates;
- missing/late frames and disconnects preserve terminal errors;
- non-XGGF devices cannot use this path to alter the XGGF audio ingestor.

**Physical acceptance:**

1. Start a recording through the existing authorized probe.
2. Press K0 once before the armed duration expires.
3. Confirm `REC` clears, `AUDIO END` arrives, and the daemon reconstructs the shortened WAV without padding it to the original duration.
4. Repeat over encrypted Wi-Fi only if the current audio baseline permits; record any pre-existing transport failure separately rather than hiding it.
5. Verify K0 without an active recording still toggles only the visualizer.

**Exit gate:** K0 stops recording safely on the physical unit, no recording can be started by this gesture, and all audio authorization and daemon ingestion regressions pass.

### Phase 4: Bounded brightness control

**Primary files:**

- `firmware-xggf/main/panel.h`
- `firmware-xggf/main/panel.c`
- `firmware-xggf/main/status.h`
- `firmware-xggf/main/status.c`
- `firmware-xggf/main/main.c`
- `firmware-xggf/main/CMakeLists.txt` if a settings module is added
- `firmware-xggf/tests/panel_test.c`
- `firmware-xggf/tests/status_test.c`

**Implementation:**

1. Add a global brightness level with four values: **2, 4, 6, 8**; 8 remains the maximum and current default.
2. Apply scaling centrally in `panel_encode`, using integer arithmetic and clamping after scaling. Do not rely on every renderer caller to adjust colors.
3. Add a small settings module with a separate NVS namespace such as `xggf_ui`; do not use the link credential namespace or change the stock NVS region.
4. Read the level once at startup, validate it, and fall back to 8 on missing/invalid data.
5. Write only when the level changes and the user selects BRIGHTNESS. Do not write NVS on every frame.
6. Display the selected level in the menu and preserve it across reboot.

**Tests:**

- every output channel remains ≤ 8 at every level;
- zero remains zero and rounding is deterministic;
- level sequence wraps 2 → 4 → 6 → 8 → 2;
- invalid persisted values fall back safely;
- no frame-rate path performs NVS writes;
- existing panel byte-encoding fixtures are updated deliberately for the selected level.

**Physical acceptance:**

- compare all levels for readability and flicker at a normal viewing distance;
- reboot and confirm persistence;
- watch for resets or heating and retain the conservative cap;
- run the standard soak with brightness changes interleaved.

**Exit gate:** bounded scaling and persistence pass host and physical tests without compromising panel stability.

### Phase 5: Mac and Linux button guidance

**Primary files:**

- `app/Sources/VibeBuddy/Korekore/Pages/DevicePage.swift`
- `app/Sources/VibeBuddy/Korekore/Pages/CharacterPage.swift`
- `app/Sources/VibeBuddy/SettingsView.swift`
- `app/Sources/VibeBuddy/ConversationView.swift`
- `app/Sources/KorekoreUI/ConversationPage.swift`
- `desktop/src/korekore/pages.rs`
- `desktop/src/main.rs`
- `app/Localization/zh-Hans.lproj/Localizable.strings`

**Implementation:**

1. Remove `device buttons` from the XGGF unsupported list. Keep Voice, Characters, Relay, and timers accurately listed as unsupported.
2. Correct comments claiming XGGF has no K0. Visible conversation text should continue to say voice commands are unsupported.
3. Add the same compact XGGF-only button guide to both Device pages:
   - left/K0: mic and identify;
   - middle/K1: pages and menu;
   - right/K2: Agent source and mute;
   - hold until the panel shows LONG for long actions.
4. Do not show generic box-only menu, Pomodoro, or voice instructions for XGGF.
5. Add zh-Hans translations on both sides through the shared localization table.

**Verification:**

- `tools/check-localization.py`
- `swift run --package-path app SelfTest`
- `cargo check -p vibebuddy-desktop`
- inspect the Mac and Linux Device pages against an identified XGGF.

**Exit gate:** both apps describe the same actual controls and unsupported features, and localization checks pass.

### Phase 6: Documentation and comprehensive regression

Update behavior documentation as implementation lands:

- `firmware-xggf/README.md` — menu, LINK, recording stop, brightness, and test instructions;
- `firmware-xggf/WORK_NOTES.md` — physical evidence and remaining limitations;
- `docs/protocol.md` — `audio.stop.request` direction, authorization boundary, and failure behavior;
- `PLAN_XGGF_64X8.md` — current capabilities, without claiming deferred voice features;
- `docs/device-menu.md` or an XGGF-specific section — gesture differences from the main box;
- Mac and Linux parity documentation if the user-visible guidance changes.

Run the final checks:

```sh
# XGGF host tests: use the exact cc commands in firmware-xggf/README.md
cargo test --workspace
swift run --package-path app SelfTest
cargo check -p vibebuddy-desktop
tools/check-localization.py

# From firmware-xggf with the documented ESP-IDF environment
idf.py build

# From the repository root, each under DEVICE.lock
flock DEVICE.lock -c 'python3 firmware-xggf/tests/physical_serial.py PORT'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K0'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K1'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K2'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/soak.py PORT 15'
```

Also run focused encrypted Wi-Fi checks for LINK, K2, and recording stop. Do not treat unresolved long-run Wi-Fi audio reliability as fixed by this stage.

Check `git status` after all builds/tests. Remove only generated files created by this work; leave pre-existing user files and unrelated changes untouched.

## 6. Physical acceptance matrix

| Area | USB | Encrypted Wi-Fi | Offline/reboot |
|---|---|---|---|
| Menu open/navigation/close/timeout | Required | Required | State resets safely |
| MIC and identify | Required | Required | Mute and visualizer behave as documented |
| LINK values | Required | Required | Correct NOT SET/NO IP/offline states |
| K2 source event | Required | Required | Reconnect preserves existing routing |
| Recording stop | Required | Required where the current audio baseline permits | No phantom session or retained PCM |
| Brightness | Required | Required | Selected level persists |
| Pairing security | No mutation | No mutation | Existing pairings unchanged |
| Soak | 15 minutes | Relevant link active | No reset, leak, flicker, or stack regression |

## 7. Rollout and rollback

1. Land and verify menu/LINK first; they are local and non-destructive.
2. Add recording stop with daemon tests before enabling it in the physical build.
3. Add brightness only after menu/LINK and recording-stop acceptance.
4. Update apps and docs with the behavior they actually ship.
5. Keep each phase independently revertible. If recording stop destabilizes audio, disable only the new K0 request branch and retain the rest of the menu.
6. If panel stability or power behavior regresses, revert brightness scaling to the fixed level 8 without affecting LINK or recording stop.
7. Reflash the previous known-good XGGF image if needed; the private full-stock backup remains for stock restoration and must not be committed.

## 8. Assumptions and decisions

- K1 long changes from “return home” to “open menu”; normal page cycling still provides direct navigation and CLOSE restores the previous page.
- The existing one-second long-press threshold and left/middle/right mapping remain unchanged.
- K0 identify remains assigned until a separate, fully supported voice-availability contract explicitly replaces it.
- Physical K0 stop is treated as a safe recording-control operation, not as authorization to start or access audio.
- Pairing and network configuration remain app/USB responsibilities.
- Brightness persistence uses a separate UI settings namespace and writes only on user changes.
- No Mac/Linux feature beyond accurate button guidance is required for this stage.

## 9. Completion criteria

This stage is complete when all of the following are true:

1. All six existing gestures still work, and K1 long opens a fully navigable menu.
2. LINK accurately reports non-secret status on USB, encrypted Wi-Fi, and offline.
3. K0 safely stops an active recording with daemon and physical evidence; it cannot start one.
4. Brightness cycles through four bounded levels and survives reboot without destabilizing the panel.
5. Mac and Linux show accurate, translated XGGF button guidance.
6. Host tests, workspace tests, app checks, localization, C3 build, physical protocol tests, relevant Wi-Fi checks, and the 15-minute soak pass.
7. Documentation matches shipped behavior, deferred features remain explicit, and no credentials or audio content appear in logs or committed artifacts.
