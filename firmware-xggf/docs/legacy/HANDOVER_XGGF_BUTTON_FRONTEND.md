# Handover: XGGF three-button frontend

**Date:** 2026-11-01  
**Status:** Incomplete. This is a handover for independent review, not a completion claim.  
**Source plan:** `PLAN_XGGF_BUTTON_FRONTEND.md`

## Worktree and device state

- The worktree was already dirty when I started. I did not reset, overwrite, or commit existing work. Review the complete current diff; do not assume every pre-existing modification came from this task.
- My changes are uncommitted.
- The last firmware image was built and flashed to `/dev/ttyACM0`, an ESP32-C3 revision 0.4 with MAC `ac:a7:04:8f:fd:30`. Esptool hash verification passed.
- Last firmware binary size: `0xf48c0` bytes in a `0x100000` app partition, with `0xb740` bytes (4%) free.
- The actual daemon executable, `target/debug/vibebuddyd`, was rebuilt before the last daemon and hardware tests. Earlier probes had used a stale executable because `cargo test` rebuilt the test harness but not that binary.
- USB protocol and encrypted Wi-Fi tests passed after the final flash, so the device was responsive at that point.
- The 15-minute soak was started and then killed at the user's handover request. It did not complete. The background task was stopped to release `DEVICE.lock`.
- No test is intentionally left running.

## What I implemented

### Firmware menu and button feedback

Files: `firmware-xggf/main/status.{h,c}`, `main.c`, `buttons.py`, `status_test.c`, `preview.c`.

- Added a hardware-independent six-row menu: MIC, LINK, BRIGHTNESS, IDENTIFY, MUTE, CLOSE.
- K1 long opens on MIC; K1 advances/wraps; K0 activates; K2 closes; LINK returns to the menu first; K2 long still toggles mute; 30-second timeout; Agent events close the menu.
- Active recording keeps rendering priority. K0 during recording bypasses ordinary menu/mic handling.
- Added menu diagnostics such as `MENU ROW ...` and `MENU CLOSED`.
- Kept the newer per-key purple DOWN dots and added a LONG text overlay at the existing one-second threshold. This deliberately changes the earlier working-note behavior in which LONG text had been removed; an external reviewer should confirm that this matches the desired product behavior.
- Added host previews for menu, LINK, brightness, and LONG.

### Read-only LINK

Files: `firmware-xggf/main/link_status.h`, `wifi_link.{h,c}`, `status.{h,c}`, `main.c`.

- Added a bounded value snapshot containing SSID, Wi-Fi state, IPv4, paired count, and encrypted-LAN-peer state. No password or private-key pointer is exposed.
- Tracks USB versus LAN from the last valid protocol envelope and uses the existing link timeout for OFFLINE.
- Added LINK pages for transport, Wi-Fi state/SSID, IP, and pair count.
- Values are clipped to the existing eight-character text area. In particular, a full IPv4 address or long SSID is not fully visible. An external reviewer should judge whether this satisfies the intended LINK requirement or needs a different renderer.

### Stop-active-recording request

Files: `firmware-xggf/main/audio.{h,c}`, `main.c`, `daemon/src/audio.rs`, `daemon/src/main.rs`, `docs/protocol.md`.

- K0 during active recording emits only `{"version":1,"event":"audio.stop.request"}` over USB and encrypted Wi-Fi.
- Requests are deduplicated. REC remains until the normal AUDIO END path finishes.
- The firmware falls back to the existing bounded local `audio_stop()` after two seconds without a stop response and reports `AUDIO STOP TIMEOUT`.
- The daemon handles the event only for an identified XGGF and an active audio session, marks early finish as allowed, and sends `device.audio.stop`. Idle, duplicate, completed, and non-XGGF requests do not trigger another stop command.
- The request cannot start, retrieve, or delete audio and does not change HTTP bearer-token/opt-in gates.
- Moved pending/timeout state into `audio.c` and added exact C tests for idle, duplicate, response, two-second timeout, and timer wraparound.
- Added `firmware-xggf/tests/audio_stop_emulated.py`, which drives the real daemon through a PTY-emulated XGGF and real authorized HTTP start. It validates one stop response for duplicate requests, a shortened 20 ms WAV, no padding to the armed duration, and no PCM frame lines in daemon logs.

### Brightness

Files: `firmware-xggf/main/panel.{h,c}`, `settings.{h,c}`, `main.c`, `main/CMakeLists.txt`, `panel_test.c`.

- Added levels 2, 4, 6, 8, with 8 as default/maximum.
- Central integer scaling in `panel_encode`, after clamping input channels to 8; zero remains zero and rounding is deterministic.
- Added the separate `xggf_ui` NVS namespace. Startup reads once; missing/invalid values fall back to 8. Menu changes write only on user selection and failed writes leave the current level unchanged.
- Host tests cover every encoded channel for input bytes 0–255 at all four levels, wraparound, and invalid fallback values.

### Mac and Linux guidance

Files: Mac Device/Conversation views, Linux Device/Conversation views, and `app/Localization/zh-Hans.lproj/Localizable.strings`.

- Added matching XGGF-only guidance for K0 mic/stop and identify, K1 pages/menu, K2 source/mute, and LONG.
- Added the guide to both standard and korekore Device layouts.
- Corrected stale comments claiming XGGF has no K0. Visible voice-unsupported text remains.
- Existing capability text already said buttons worked, so no unsupported-list code change was needed there.
- Added shared zh-Hans entries; localization now reports 607 keys.

### Documentation and interactive probes

- Updated `firmware-xggf/README.md`, `docs/protocol.md`, `docs/device-menu.md`, `PLAN_XGGF_64X8.md`, and `docs/platform-parity.md`.
- Added optional `--button-stop` handling to `firmware-xggf/tests/audio_probe.py` and audible action cues to it and `tests/buttons.py`.
- The audible cue is a generated 1 kHz WAV played with `aplay`; direct playback succeeded and the user confirmed hearing it. Startup/action beeps do not make the interactive physical stop test automated.

## Verification that passed

### Host and build checks

- XGGF panel C test: passed.
- XGGF status/renderer C test: passed, including menu, LINK, LONG, recording priority, and state transitions.
- XGGF font C test: passed.
- XGGF buttons C test: passed.
- XGGF audio C test: passed, including the new stop-request timeout/wrap tests.
- PBM preview generation: passed; LINK USB, LONG, and BRIGHT 2 PNG conversions were visually inspected by the agent.
- `cargo test --workspace`: **432 passed, 3 ignored, 0 failed**.
- Focused daemon stop and audio ingestion tests: passed.
- `cargo check -p vibebuddy-desktop`: passed.
- `tools/check-localization.py`: passed, **607 keys**.
- ESP-IDF 5.5.4 C3 build: passed repeatedly, including the final source state.
- `python3 -m py_compile` for the modified/new Python probes: passed.

### Automated daemon end-to-end check

`python3 firmware-xggf/tests/audio_stop_emulated.py target/debug/vibebuddyd` passed after rebuilding the actual daemon:

- real PTY serial device emulation;
- real XGGF identity/pairing flow;
- real authorized HTTP recording start;
- AUDIO BEGIN/FRAME input;
- duplicate `audio.stop.request` events;
- exactly one `device.audio.stop` response;
- valid 20 ms WAV rather than the armed ten seconds;
- zero timing padding and zero lost frames;
- no `AUDIO FRAME` lines in the daemon log.

### Physical checks that passed

- Final flash and esptool hash verification: passed.
- `physical_serial.py`: passed after the final flash (board identity, hello, states, malformed and oversized input).
- Guided K0/K1/K2 tests eventually passed: visualizer/identify, menu MIC/LINK and K2 close suppression, source event, mute/unmute restored off. There were failed/interrupted attempts before these passes.
- `wifi_smoke.py`: passed; scan, USB pairing/provisioning, Wi-Fi address, mDNS/encrypted reconnect, and all eight control acknowledgements.
- Ordinary `audio_probe.py --quick`: passed after rebuilding the daemon. USB 8 kHz, USB 16 kHz, and encrypted-Wi-Fi 16 kHz each produced exact five-second WAVs with zero lost/duplicate/reordered frames and zero device overruns.

## Failures and mistakes during this task

- The first interactive K0 recording-stop attempt recorded the full ten seconds because no press occurred.
- A later interactive attempt ended with `unknown audio record` and did not produce a shortened WAV.
- Several isolated-daemon attempts timed out waiting for `/v1/status` to show pairing. One log showed PAIRED arriving just after the first timeout; extending the timeout did not make the interactive probe a reliable acceptance path.
- The initial beep implementation did not sound because those probes exited before reaching the beep call. I then generated and played the WAV directly; the user confirmed that sound worked.
- The emulated stop probe initially failed to get `device.audio.stop` because the daemon executable was stale. Rebuilding `target/debug/vibebuddyd` fixed that specific cause, and the same probe then passed twice.
- A daemon unit test initially caught duplicate stop responses. I changed `AudioIngestor::stop()` to report only the first active transition; the test then passed.
- LONG renderer tests caught underlying-label bleed and dot overlap. I cleared the label area before drawing LONG and adjusted the test to ignore separately tested dot pixels; the host suite then passed.
- I spent too long on interactive physical probing instead of using the deterministic PTY end-to-end probe earlier. The interactive physical K0 stop acceptance was never completed.

## Not verified / incomplete

- **Physical K0 early-stop acceptance was not completed.** Do not claim that pressing the real K0 produced a shortened WAV. The firmware C logic, daemon unit tests, and PTY end-to-end path passed, and all six ordinary physical gestures passed, but that specific end-to-end physical scenario remains unproven.
- The optional interactive `audio_probe.py --button-stop` mode did not pass.
- The final 15-minute soak did not complete; it was cancelled for this handover.
- Brightness scaling/persistence passed host tests, but physical persistence across reboot and subjective readability/flicker at all four levels were not separately verified.
- LINK rendering and state transitions passed host tests and encrypted Wi-Fi controls passed, but a complete physical visual LINK acceptance matrix over USB, LAN, unplug/offline was not completed.
- Mac compilation and UI inspection were not performed because this host has no Swift toolchain/Xcode. Linux compilation and localization passed; neither platform's Device page was visually inspected against a live XGGF in this task.
- No measured panel waveform or new power/thermal measurement was performed.
- Long-run Wi-Fi audio reliability was not redesigned or claimed fixed; only the quick physical recordings listed above passed.

## Suggested independent review order

1. Review the complete diff and separate this task's changes from pre-existing worktree changes.
2. Judge the LONG-overlay and eight-character LINK clipping decisions against the intended contract.
3. Re-run the automated host/workspace/emulated checks on the exact deliverable.
4. If physical acceptance is still required, use the rebuilt daemon and test real K0 early stop, brightness persistence, LINK pages, and the 15-minute soak without treating the earlier interactive failures as passes.
5. Run Mac SelfTest/UI checks on a suitable Swift/Xcode host.
