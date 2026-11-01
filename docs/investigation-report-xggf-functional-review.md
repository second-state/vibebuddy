# Investigation Report: XGGF functional review

**Date:** 2026-11-01
**Branch / revision reviewed:** `xggf-64x8`, through `6e3c494` (`Add an XGGF connection bar and recording dot`)
**Scope:** `firmware-xggf/`, daemon device/audio paths, Mac and Linux recorder UI, protocol and product documentation, and the recorded hardware-verification notes. This is a code and document review; hardware claims below are attributed to the working notes and probe report, not independently rerun.

## Summary

The branch has advanced well beyond the initial 64×8 status display: it now has paired encrypted Wi-Fi, button and buzzer support, microphone visualization, bounded speech capture and transfer, and explicit network/host indicators. The central functional flows are present, and the implementation shares the established Noise/LAN stack rather than adding a cleartext network control path.

The most consequential review findings are at the edges of those flows:

1. When the opt-in audio gate is enabled, the audio HTTP routes accept requests without a token. That is safe only while the daemon is reachable exclusively by trusted local processes; `VIBEBUDDY_BIND` can bind it to a non-loopback interface.
2. On macOS, leaving the recorder view cancels its polling task but does not stop the device or daemon recording, despite the view's comment saying it does. The recording can continue for its configured duration (up to 60 seconds).
3. Linux's Korekore recorder still hides `lost_frames`, so it can report progress without showing degraded audio that the standard Linux and Mac recorders expose.
4. The recovery instructions contradict the hardware log: `firmware-xggf/README.md` says full stock restoration has not been tested, while `WORK_NOTES.md` records that the verified image was restored and the user confirmed the stock display returned.

Other findings concern undocumented protocol behavior and feature omissions. No finding below indicates an observed device damage or an unauthenticated device-side Wi-Fi link.

## Findings

### F1 — Opt-in audio routes are unauthenticated on non-loopback daemon binds

**Severity: High when the daemon is exposed to an untrusted network; otherwise conditional.**

- `daemon/src/main.rs:253-256` accepts `VIBEBUDDY_BIND` and defaults to `127.0.0.1:7331`; it does not restrict non-loopback addresses.
- `daemon/src/main.rs:839-847` makes `AudioGate::OptIn` authorize every request without inspecting headers. The same authorization check gates start, stop, WAV retrieval, stats, and discard (`:874-935`). `VIBEBUDDY_XGGF_AUDIO=1` selects this mode (`:859-862`).
- The audio is microphone content; this gate therefore allows any client that can reach a non-loopback daemon listener to start capture and retrieve the resulting WAV when the operator enabled the feature. There is no per-request credential in OptIn mode.
- Probe-token mode does require a bearer token. The default loopback bind also materially limits exposure, but it is not enforced when the bind override is used.

**Recommended follow-up:** Either reject `OptIn` when binding to a non-loopback address, require a per-install local token for these routes even in opt-in mode, or document and enforce that the opt-in deployment must remain loopback-only. Add a test for the chosen boundary. Keep the device-side Noise pairing/authentication as-is.

### F2 — Leaving the Mac recorder view stops polling but not recording

**Severity: Medium; bounded capture continues after the UI leaves.**

- `app/Sources/VibeBuddy/AudioRecorder.swift:58-60` says the `.task(id: recording)` follows the recording and that leaving the page cancels it.
- The task's cancellation only exits `follow()`'s polling loop (`:82-110`). There is no cancellation handler or request to `stopAudio()` / `discardAudio()` on disappearance. `stopAudio()` is only called by the Stop button (`:27-31`).
- The daemon and firmware allow recordings up to 60 seconds (`daemon/src/audio.rs:68-73`; `firmware-xggf/main/audio.c:32-43`). Thus leaving the view can leave the microphone active and let the daemon complete and hold the WAV in memory, while the view that could display or discard it is gone.
- This is not a disk-retention issue: the daemon's buffer is in memory. It is a mismatch between the stated view lifecycle and actual capture lifecycle, and can surprise users who navigate away.

**Recommended follow-up:** On view disappearance, explicitly stop or discard an active recording and await/cancel that request safely; alternatively keep the recorder state outside the transient view and revise the comment/UI so ongoing capture remains visible and controllable. Cover navigation away during an active recording on both platforms.

### F3 — Linux Korekore hides the recorder's lost-frame count

**Severity: Medium; a degraded capture can look healthy in one UI.**

- `desktop/src/korekore/pages.rs:750-755` renders only `frames of expected` while recording.
- Linux's standard page renders `stats.lost_frames` when nonzero (`desktop/src/main.rs:2452-2462`); the Mac recorder does too (`app/Sources/VibeBuddy/AudioRecorder.swift:62-66`).
- The daemon tracks and returns `lost_frames` (`daemon/src/audio.rs:15-24, 212-218, 275-281`). The data is available at the Korekore call site but is omitted there.

**Recommended follow-up:** Mirror the conditional lost-frame label in the Linux Korekore recorder and add a UI/state test for a nonzero loss count. This is a direct Mac/Linux parity gap under the repository instructions.

### F4 — Stock recovery status conflicts between the README and working notes

**Severity: Medium; creates uncertainty around the safety procedure.**

- `firmware-xggf/README.md:43-51` describes the full-flash restore steps, then states that this procedure “has not been tested on the current target unit.”
- `firmware-xggf/WORK_NOTES.md:22-27` records that the verified image was written back with hash verification, stock firmware was restarted, and the user confirmed the normal display returned.
- These cannot both describe the same target and restore procedure accurately. The README wording could cause someone to repeat a destructive operation under a false assumption, or undermine confidence in an actually validated recovery path.

**Recommended follow-up:** Reconcile the README with the exact evidence. State precisely which image was restored, how it was verified, and which stock functions were checked. Retain the per-unit warning that another unit's dump is not interchangeable.

### F5 — XGGF silently drops malformed, oversized, and unknown events despite the shared protocol contract

**Severity: Low to medium; mainly affects diagnosis and future clients.**

- `docs/protocol.md:10, 26` says oversized lines receive `ERROR input_too_large`, unknown events are reported as `EVENT`, and unsupported protocol versions receive `ERROR unsupported_version`.
- XGGF silently returns for malformed JSON or incorrect field types (`firmware-xggf/main/main.c:169-177`), silently ignores unhandled events (`firmware-xggf/main/status.c:20-26`), and drops oversized input without a response (`main.c:255-267`).
- The physical serial test intentionally exercises malformed/oversized recovery (`tests/physical_serial.py`), but that proves framing recovers, not that a caller receives the documented protocol response.
- The branch's initial plan allowed unsupported commands to be ignored, so this may be a deliberate implementation boundary; the shared protocol document does not qualify its contract for this board.

**Recommended follow-up:** Either implement the shared replies where safe or document the XGGF-specific behavior and pin it with protocol tests. Do not emit success acknowledgements for events the firmware does not handle.

### F6 — `buddy.say` and daily greetings are invisible on XGGF

**Severity: Low; user-visible omission is not explained.**

- The daemon sends daily greetings after a device reports its build (`daemon/src/main.rs:430-434`); the standard firmware handles `buddy.say`.
- XGGF handles its supported events through `status_event()` and the explicit `device.*` paths; `buddy.say` falls through without a visible result (`firmware-xggf/main/main.c:195-205`, `firmware-xggf/main/status.c:20-26`).
- The XGGF README says voice is unsupported, but does not say that the greeting event is also ignored. Unlike recorded announcements, a greeting's text could in principle be displayed even without a speaker/TTS path.

**Recommended follow-up:** Either render a suitably short greeting, suppress greetings for XGGF, or document the no-op. Avoid treating receipt as proof of a rendered greeting.

### F7 — Shared protocol and product docs lag the implemented XGGF/audio surface

**Severity: Low; implementation and third-party expectations can drift.**

- `docs/protocol.md` does not describe the new `BOARD`, `STATE`, `AUDIO BEGIN/FRAME/END`, `MIC`, `SENSOR`, `BUTTON`, `LINK DIAG`, or `WIFI SEEN` lines, or the `device.wifi.scan` and audio event schemas. The detailed audio contract is instead in `firmware-xggf/AUDIO_PROBE_REPORT.md` and code.
- The root README's API endpoint catalog does not list `/v1/device/audio*`, and its repository layout does not identify `firmware-xggf/`; its Chinese counterpart should stay in step if those sections are changed.
- The useful XGGF-specific behavior is described in `firmware-xggf/README.md`, but the shared protocol document is the natural contract for daemon/device implementers.

**Recommended follow-up:** Add a board-scoped protocol section and audio endpoints/events to the shared docs; update the bilingual README together. Mark experimental/unsupported events explicitly.

### F8 — XGGF is absent from the formal parity scenarios and simulator

**Severity: Low; future regressions may evade routine checks.**

- `docs/platform-parity.md` and `docs/parity-scenarios/` have no XGGF recorder, button, or display-indicator scenarios. The simulator does not model the XGGF board/audio capture path.
- The new functions span C firmware, daemon transport, and both UIs. Existing physical scripts and host tests cover much of the firmware flow, but do not give the formal parity suite a repeatable view of XGGF-specific app behavior.

**Recommended follow-up:** Add focused parity scenarios for audio consent and teardown, lost-frame display, unsupported firmware/update controls, and the limited 64×8 status/connection display. Extend the simulator only where it can reproduce the relevant semantics without pretending to emulate hardware fidelity.

## Verified consistencies and strengths

- **Agent-state mapping:** XGGF uses the same 15-second link timeout, five-second done display, and core `task.start`, `agent.idle`, `agent.input_required`, `task.done`, `task.error`, and `agent.blocked` mapping as `firmware-rs/core/src/firmware.rs`; wrap-safe unsigned elapsed-time checks are used in `status.c:34-40`.
- **Network security:** Wi-Fi TCP uses the Rust bridge around the existing shared `Link`/`Lan` Noise implementation (`firmware-xggf/link/src/lib.rs`, `firmware-xggf/main/wifi_link.c`). USB priority, pairing, replay handling and encrypted LAN are reused rather than replaced with an unauthenticated control protocol.
- **Firmware safety gates:** XGGF announces `BOARD xggf-64x8` before the build marker (`main.c:62-73`); daemon update routes reject unsupported boards; app update controls are gated; and the ROM flasher checks the ESP32-S3 chip magic. Preserve announcement ordering and all gates.
- **Buttons:** K2 short press emits the standard `button` JSON consumed by the daemon, while K2 long press toggles mute. The XGGF-specific K0/K1 actions are documented in `firmware-xggf/README.md` and the working notes say the physical mapping was verified.
- **Bounded audio capture:** Device capture is explicitly requested, capped at 60 seconds, visibly marked on the panel, and uses bounded buffering. The daemon reconstructs audio in memory, validates framing and records losses; probe notes report no default disk retention. The physical end-to-end and disruption results are recorded in `AUDIO_PROBE_REPORT.md` and `WORK_NOTES.md`.
- **Display indicator semantics:** The new right-edge column distinguishes “no configured Wi-Fi / configured without an address / station has an address” from the separate daemon-link state. It does not claim the green Wi-Fi segment means internet reachability.

## Evidence limits and follow-up order

This review inspected the current committed source and documentation and compared the recorded hardware notes. It did not flash the device or rerun the physical audio, Wi-Fi, soak, or recovery tests. Hardware results therefore have the confidence of the cited reports, not independent reproduction. The work notes also say the final font and connection-indicator display changes still awaited on-panel confirmation at the time of those entries; the current branch log alone does not close that validation gap.

Suggested order before treating the recorder as a user-facing feature:

1. Close the non-loopback audio authorization boundary (F1).
2. Make view/navigation teardown stop or retain visible control of active capture (F2).
3. Fix the Linux Korekore loss display (F3).
4. Reconcile stock recovery documentation (F4).
5. Align protocol/docs and make ignored greeting behavior explicit (F5–F7), then add parity coverage (F8).

## Implementation status after this review

This follow-up reflects the working tree on 2026-11-01 after the review revision above. It is a status addendum; the original findings describe the reviewed snapshot.

| Finding | Follow-up status |
|---|---|
| F1 audio authorization | Addressed in `daemon/src/main.rs`: app opt-in is disabled with a warning on non-loopback binds; loopback and the bearer-token probe mode remain available. Focused IPv4/IPv6/wildcard/token test passed. |
| F2 capture lifecycle | Addressed in Mac and Linux UI code: active capture is discarded when leaving the recorder, and generation guards prevent late start/poll replies from reviving it. Linux checks passed; Mac compilation/runtime validation remains unavailable on this host because Swift/Xcode is absent. |
| F3 Linux Korekore loss display | Addressed: it now displays nonzero `lost_frames`, matching the other recorder views. Linux `cargo check` passed. |
| F4 recovery instructions | Addressed: the XGGF README now states the recovery verification actually recorded for this unit and limits the claim to this examined device. The current restore/reinstall round trip passed esptool write verification and post-reinstall XGGF tests. |
| F5 silent-drop contract | The chosen behavior remains silent drop for unsupported/malformed XGGF input; `docs/protocol.md` now states the board-specific deviation and parser limit explicitly. |
| F6 `buddy.say` | The event remains unsupported and has no visible effect; `docs/protocol.md` now calls out the no-op. Whether to suppress the greeting or render a bounded text indication remains a product decision. |
| F7 protocol/product docs | Addressed in `docs/protocol.md`, both root READMEs, and `firmware-xggf/README.md`; English/Chinese README sections were updated together. Localization check passed (602 keys). |
| F8 parity coverage | Added DEV-19 and linked it from the scenario index and platform-parity matrix. The simulator still does not emulate XGGF hardware/audio timing; no misleading hardware simulation was added. |

Post-change checks: XGGF panel/status/font/button host tests, six font-generator tests, daemon focused authorization test, Linux app check, physical USB protocol, daemon board/update guard, encrypted Wi-Fi/mDNS control smoke, Wi-Fi persistence, and a two-minute physical soak passed. The current firmware image leaves 5% of the smallest app partition free. The newest button-dot/font/connection-bar rendering was flashed and soaked, but its visual legibility was not independently confirmed; the full daemon suite, Mac build, audio capture end-to-end regression after the UI changes, and measured panel waveform remain open validation items.
