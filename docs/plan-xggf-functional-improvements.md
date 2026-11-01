# Plan: Improve VibeBuddy support for the XGGF device

**Purpose:** Turn the highest-impact findings in [`investigation-report-xggf-functional-review.md`](investigation-report-xggf-functional-review.md) into a safe, testable improvement sequence for the XGGF PixelClock and the VibeBuddy daemon and apps.

**Product priority:** Make this intact device a reliable VibeBuddy notification endpoint over its existing paired, encrypted Wi-Fi link. Treat the microphone recorder as an optional, explicitly controlled feature. Preserve the device's existing recovery path and do not require disassembly.

**Baseline caution:** The review describes branch `xggf-64x8` through `6e3c494`. The branch has since advanced to `eaa2ed5`, and the worktree contains uncommitted button-display changes. Reconcile the live tree and working notes before implementation; do not overwrite or discard that work.

## Outcomes

1. Optional audio capture is unreachable to other network clients when enabled for app use.
2. Capture lifetime matches what the Device page tells the user, and users can see and stop it from both Mac and Linux.
3. Every recorder UI reports frame loss and capture errors consistently.
4. XGGF recovery and supported-protocol instructions agree with tested evidence.
5. Device and daemon behavior is explicit for supported and unsupported events, including the daemon greeting.
6. XGGF behavior has repeatable host, daemon, app-parity, and physical acceptance checks.
7. The established encrypted Wi-Fi notification path, board identification, and incompatible-firmware protections remain intact.

## Work sequence

### Phase 0 — Reconcile the current branch and preserve evidence

Before changing code:

- Record `git status`, `git log`, and the latest XGGF work-note entries. Keep existing uncommitted changes intact and identify which review findings have already been addressed by newer work.
- Re-check the current implementations behind F1–F8 below. In particular, verify the latest audio UI lifecycle, `AudioGate` policy, Linux Korekore progress, recovery instructions, protocol docs, and test coverage.
- Confirm that the private, device-specific stock image and the tested restore evidence still exist in their protected locations. Do not print or copy credentials from the image.
- Preserve the current board-first boot announcement, shared Noise/LAN link, USB priority, and all app/daemon/ROM-flasher firmware safety gates.

**Exit condition:** A short implementation checklist names each still-open issue against the current revision and identifies relevant existing tests. No source changes are needed for this phase.

### Phase 1 — Close the optional audio network exposure (review F1)

The app opt-in currently authorizes audio HTTP requests without a bearer token, while the daemon permits a configurable bind address. Choose and enforce a boundary before further promoting recording.

**Recommended policy:** Keep the app opt-in convenient for the local Device page, but permit `VIBEBUDDY_XGGF_AUDIO=1` only when the daemon binds to a loopback address. Fail closed with a clear startup error if opt-in audio and a non-loopback bind are configured together. Keep the probe-token mode for controlled diagnostics, with a strong random token and explicit operator-managed reachability.

Implementation tasks:

- Validate the parsed `SocketAddr` after reading `VIBEBUDDY_BIND`; account for IPv4 and IPv6 loopback. Treat wildcard and LAN addresses as non-loopback.
- Ensure the daemon refuses to start, or disables OptIn audio with a prominent diagnostic, when the policy is violated. Prefer a fail-closed behavior that does not leave users thinking capture is enabled safely.
- Keep authorization checks common to all audio endpoints: start, stop, get WAV, get stats, and discard.
- Document the interaction between audio opt-in and bind address in the root README and XGGF README. Preserve the default loopback bind.
- Add daemon tests for default loopback, IPv4 loopback, IPv6 loopback, wildcard/non-loopback rejection, and token-gated probe requests. Check that all audio endpoints share the same gate.

**Acceptance criteria:** With OptIn mode and a loopback bind, the Mac/Linux Device pages can still record, poll, play, save, and discard. With OptIn and a non-loopback bind, the daemon does not expose any audio route. Probe-token authorization remains required when that mode is selected. No PCM or WAV bytes appear in logs.

### Phase 2 — Make capture lifetime explicit and safe (review F2)

The current Mac recorder's view task stops polling when it disappears but does not send a stop or discard command. Decide one shared lifecycle for both apps.

**Recommended behavior:** Leaving the recorder page while capture is active sends the existing discard request (`DELETE /v1/device/audio`), stops the device, and drops the in-memory result. The explicit Stop button remains a normal early finish that keeps the partial WAV available. The normal duration cap remains 60 seconds as a final safety bound.

Implementation tasks:

- Define capture states for idle, starting, recording, stopping/discarding, completed, and failed. Avoid interpreting “request accepted” as proof the device has begun recording.
- In the Mac app, move request ownership to a lifecycle-aware model/coordinator or add reliable disappearance cancellation. Avoid relying only on SwiftUI `.task` cancellation, which currently cancels the poll loop but not capture.
- In the Linux app, verify behavior when navigating between pages, closing the window, stopping the desktop process, and losing daemon connectivity. Apply the same stop/discard policy as Mac.
- Ensure a late poll response cannot replace a discarded result or restart the UI's recording state. Ensure repeated Stop/Discard and navigation events are safe and idempotent.
- Keep the visible `REC` marker synchronized with actual device capture. On a lost link, report a terminal result and let the firmware's existing bounded recording finish/cleanup path take effect.
- Verify whether the daemon should maintain an owner/liveness lease in addition to explicit UI teardown. Add a lease only if the client lifecycle cannot guarantee cleanup; keep its timeout longer than expected polling/network jitter and shorter than the 60-second capture cap.
- Update stale comments and user-facing copy to describe what navigation does.

**Acceptance criteria:** Navigating away during a recording on either platform stops capture and discards its data within a defined bound; explicit Stop produces a playable partial WAV; Discard clears completed data; daemon/device loss produces a terminal, non-stuck state; a recording never exceeds the device's configured maximum duration. Test request races and page changes, not only the happy path.

### Phase 3 — Make recorder integrity visible in every app view (review F3)

- Add the conditional “N lost” count to the Linux Korekore recorder, matching Linux standard and Mac behavior.
- Show terminal audio errors and device overruns/discontinuities consistently. Use existing stats where available and do not label padded zero samples as captured speech.
- Keep the wording, state transitions, and Chinese localization aligned across Mac, Linux standard, and Linux Korekore views.
- Add focused UI/state coverage for clean recording, nonzero daemon `lost_frames`, device overrun/discontinuity, early Stop, and a terminal transport error.

**Acceptance criteria:** Given equivalent daemon status, all three views show equivalent recording progress, loss information, and terminal state. No view presents a lossy recording as clean.

### Phase 4 — Correct the recovery and operator instructions (review F4)

- Compare `firmware-xggf/README.md` with the actual work-note evidence for the examined unit: complete image verification, restoration write/read verification, and the user's confirmation that stock firmware booted.
- Replace the contradictory “not tested” statement only to the extent the evidence supports. Distinguish the restore procedure tested on this unit from stock functions not exhaustively tested.
- Keep the critical warnings: the backup is unit-specific, can contain credentials, belongs outside version control, and another XGGF must not use it as its recovery image.
- Clarify the exact partition-layout consequences of the experimental image and the conditions needed for ROM download-mode recovery. Never advise eFuse erasure.
- Reconcile README steps with the actual scripts/tool versions and the `DEVICE.lock` physical-test discipline. Do not include Wi-Fi passwords, private keys, or dump hashes that reveal private device state.

**Acceptance criteria:** A reader can distinguish verified steps from unverified assumptions and can follow the examined unit's recovery procedure without mistaking it for a universal XGGF image. The working notes and README no longer contradict each other.

### Phase 5 — Define XGGF protocol behavior and eliminate silent ambiguity (review F5–F7)

- Compare each incoming event the daemon can send against XGGF handlers. Make a capability table for supported, intentionally ignored, and rejected events.
- Decide how XGGF responds to malformed JSON, unsupported protocol versions, unknown events, and oversized lines. Either implement the documented v1 error replies safely or make the board-specific deviation explicit in `docs/protocol.md` and tests. Never acknowledge an event as handled unless its behavior occurred.
- Decide how `buddy.say` behaves. Preferred options are to suppress it for XGGF in the daemon, or render a short, bounded text indication if the product wants greetings on the tiny display. Document the choice and ensure a greeting does not overwrite a higher-priority Agent state or recording indicator.
- Document XGGF-specific board discovery and diagnostic lines (`BOARD`, `STATE`, `WIFI SEEN`, `MIC`, `SENSOR`, `BUTTON`, `LINK DIAG`) and the `device.wifi.scan` flow.
- Specify the audio control events and `AUDIO BEGIN/FRAME/END` framing, sequence, flags, maximum sizes, timeout, loss handling, and endpoint behavior in the shared protocol documentation, linking to the deeper measurement report.
- Add `/v1/device/audio*` to the daemon endpoint catalog in `README.md` and `README.zh-CN.md`; keep their sections aligned. Add `firmware-xggf/` to repository layout documentation if that layout remains maintained.
- Preserve the 4,608-byte line cap or change it only with an explicit compatibility decision; the current shared contract's 1,024-byte limit applies to devices without transfer mode. State clearly which bound XGGF enforces.

**Acceptance criteria:** A protocol implementer can predict each XGGF response and limit from shared docs. Tests pin the chosen replies/no-op semantics, and daemon acknowledgements are not mistaken for render acknowledgements.

### Phase 6 — Add sustainable XGGF parity and regression coverage (review F8)

- Add focused parity scenarios for XGGF device identification, unavailable firmware actions, audio opt-in, page exit during capture, lost frames, terminal recording errors, and local button behavior.
- Extend the device simulator only for protocol/UI states it can represent faithfully. Keep physical properties (Wi-Fi RSSI, I2S timing, display quality, buffer headroom) in hardware tests rather than simulating them as facts.
- Add daemon tests for authorization and lifecycle races; renderer/firmware host tests for recording marker and button-dot coexistence; app tests for equivalent statuses in all views.
- Keep localization checks and Mac/Linux parity review in the implementation checklist. A visible setting or supported action must have corresponding behavior on both platforms or an explicit documented platform limitation.
- Ensure test fixtures and logs use fake SSIDs, keys, audio samples, and tokens. Never place the user's network password or device identity secret in source or docs.

**Acceptance criteria:** Every phase 1–5 regression has a repeatable automated test at the layer where the behavior is owned. Hardware-only acceptance remains a separate, clearly recorded gate.

### Phase 7 — Reconfirm the Wi-Fi notification product path and close the pilot

The user's primary product goal is notification over Wi-Fi, so the optional recorder improvements must not divert attention from the core buddy link.

- Re-run the existing physical Wi-Fi smoke test using the current device image and daemon, including USB pairing/provisioning, cache-free mDNS discovery, encrypted reconnection, state notification delivery, and removal of temporary test pairing.
- Confirm state resynchronization after device reset and daemon restart, heartbeat timeout/restore, and USB priority taking over a live Wi-Fi link. Confirm the network indicator and host-link indicator represent those separate conditions accurately.
- Test every button gesture while online and during capture; ensure K2 reaches the current Agent source, local gestures remain local, and recording feedback does not collide with the button dots or connection bar.
- Verify the current button-dot/font/connection-indicator changes on the physical panel after the latest source changes. Keep the display low-brightness and watch reset/heap/stack diagnostics during a bounded soak.
- Rebuild and reinstall the experimental firmware, then restore the unit's verified stock image and confirm stock boot. Reinstall the prototype once more and confirm it boots and re-establishes Wi-Fi without exposing credentials.
- Record exact software revision, hardware unit, test setup, test results, and remaining limitations in `firmware-xggf/WORK_NOTES.md` and the appropriate test report.

**Acceptance criteria:** Core Agent notifications continue over the paired, encrypted Wi-Fi link through reconnect/restart cases; no incompatibility/update path can flash released S3 firmware to the C3; the physical panel shows the final UI; the recovery path remains usable. Audio claims remain scoped to the measurements actually repeated.

## Ordering and release gates

| Order | Work | Gate |
|---|---|---|
| 0 | Reconcile new commits and uncommitted work | Findings still apply to the live branch; no external-agent work is overwritten |
| 1 | Audio authorization boundary | No unauthenticated audio access on non-loopback listeners |
| 2 | Capture lifecycle on Mac and Linux | Navigation, stop, discard, and disconnect have bounded, tested outcomes |
| 3 | Loss/error parity | All app views expose equivalent fidelity information |
| 4 | Restore documentation | Recovery claims match tested unit evidence |
| 5 | Protocol/product documentation | Shared contract matches firmware and daemon behavior |
| 6 | Parity coverage | Regression scenarios exercise the cross-platform path |
| 7 | Physical Wi-Fi and recovery acceptance | Device remains a reliable, recoverable notification buddy |

Do not advertise the XGGF recorder as a normal user-facing capability until Gates 1–4 pass. Keep XGGF firmware explicitly experimental until the physical acceptance gate and board-specific update/recovery story are complete.

## Out of scope for this improvement pass

- Replacing or weakening the shared Noise pairing and encrypted Wi-Fi link.
- Enabling speech recognition, cloud transcription, Relay audio, or automatic microphone capture.
- Adding calibration requirements for temperature or humidity; the user has said those readings may remain uncalibrated.
- Disassembling the device, changing its hardware, or using another unit's stock flash image.
- Expanding the feature set with speculative stock functions before the notification, recording-consent, and recovery behavior is dependable.

## Execution status — 2026-11-01

The plan was started on branch `xggf-64x8`. The original review snapshot was `6e3c494`; work was reconciled against the later branch tip `eaa2ed5` and existing uncommitted display changes were preserved.

| Phase | Status |
|---|---|
| 0 — Reconcile | Done. Kept the current uncommitted button-dot adjustments and recorded this implementation pass in `firmware-xggf/WORK_NOTES.md`. |
| 1 — Audio authorization | Done. App opt-in is disabled with a warning for non-loopback binds; IPv4/IPv6 loopback and bearer-token probes remain supported. Focused authorization test passed. |
| 2 — Capture lifecycle | Implemented on Mac and Linux. Linux `cargo check` passed; Swift/Xcode is unavailable here, so Mac compile/runtime validation remains open. |
| 3 — Integrity parity | Done for the identified UI gap: Linux Korekore now shows lost-frame counts. Linux standard and Mac already did. |
| 4 — Recovery docs | Done. XGGF README now qualifies recovery evidence to the examined unit and does not imply its image is portable. |
| 5 — Protocol/docs | Done for the documented contract: XGGF parser deviations, event/diagnostic surface, audio protocol/endpoints and greeting no-op are described; both READMEs were updated together. `buddy.say` remains intentionally unimplemented pending a product decision. |
| 6 — Parity coverage | DEV-19 and a platform-matrix row were added. The simulator was not extended because it does not faithfully model XGGF audio/display hardware. Full automated app parity coverage remains a follow-up. |
| 7 — Physical acceptance | USB protocol, daemon board/update guard, Wi-Fi scan/provisioning, cache-free mDNS discovery, eight encrypted controls, post-reinstall Wi-Fi persistence and a two-minute soak passed. The verified stock image was restored with esptool write verification, then the XGGF bootloader/partition table/app were reinstalled and Wi-Fi reprovisioned. The stock-boot display confirmation rests on the earlier user-confirmed restoration note; a fresh visual confirmation was not collected during this pass. |

Additional checks passed: firmware panel/status/font/button/audio host tests, PBM preview generation, six font-generator tests, localization check (602 keys), daemon focused test (1 passed; 433 filtered), Linux app `cargo check`, and ESP-IDF 5.5.4 build. The built application leaves 5% of the smallest app partition free. The full daemon suite and post-change audio end-to-end capture test were not rerun; the final display's visual legibility and measured SPI waveform remain unverified. Keep the firmware experimental until those gaps are addressed and its limited image headroom is resolved or accepted for a frozen build.
