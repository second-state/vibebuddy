# Daemon audio diagnostics — Gate 1 report

## Scope

This report covers the completed daemon-diagnostics subagent investigation for the XGGF speech-recording probe. The work was limited to:

- `daemon/src/audio.rs`
- `daemon/src/main.rs`

No firmware, physical-device behavior, protocol retention policy, or token-gating behavior was intentionally changed. No commit was created.

## Problem addressed

The daemon already had bounded, in-memory audio ingestion and WAV reconstruction, but terminal diagnostics were incomplete. `AudioIngestor` had a partially implemented `last_error`: some paths set or cleared it, but frame-rejection paths did not preserve their exact reason, and the statistics endpoint did not return it.

That made it difficult to distinguish malformed input, session-state errors, and disconnects from an active but incomplete recording.

## Implemented changes

### Terminal rejection reasons

`daemon/src/audio.rs:142` now preserves the exact terminal rejection reason in `last_error` when `fail()` is called. Messages remain static diagnostic text; audio payloads are not logged or included in the error.

The existing behavior for arming, cancellation, disconnects, and successful WAV retrieval remains intact:

- Arming a recording clears the previous terminal error.
- Explicit cancellation clears it.
- A device disconnect records `device disconnected during recording`.
- Successful WAV retrieval clears it.

### Statistics and active progress

`daemon/src/main.rs:889` now returns an authorized terminal recording/disconnect error as `409 Conflict`. Otherwise, the existing JSON behavior remains:

1. Completed recording statistics when available.
2. Active-session progress while a recording is underway.
3. Statistics retained from the last completed recording.

This keeps live progress available without hiding terminal failures.

### Robustness coverage

The audio test module was expanded to eight focused tests covering:

- Active progress and completed-stat retention.
- Early completion versus an explicitly requested stop.
- Cancellation and disconnect recovery.
- Exact malformed, oversized, invalid-base64, length, and synchronization errors.
- Gap zero-filling and ignored duplicate/reordered payloads.
- Timing padding and trimming, including the resulting WAV content.

## Verification

The subagent reported:

```sh
cargo test --manifest-path daemon/Cargo.toml audio::
```

Result: **8 passed, 0 failed**.

```sh
cargo test --manifest-path daemon/Cargo.toml
```

Result: **428 passed, 0 failed, 3 ignored**.

`git diff --check -- daemon/src/main.rs` was clean.

Independent checks during the same development stage also passed for firmware audio, panel, renderer, Python syntax, localization, and the Linux desktop build.

## Privacy and retention

- Audio remains in memory by default.
- No audio-content logging was added.
- Token gating remains required for the probe endpoints.
- WAV reconstruction and protocol behavior were not changed by this diagnostics work.

## Limits of this report

The subagent ran no hardware or USB tests. It did not diagnose or fix the intermittent encrypted Wi-Fi recording failures and did not run a long Wi-Fi stress capture. Those remain separate Gate 1/2 work.

The report also does not claim that subjective speech intelligibility has been confirmed. Existing WAVs were played through the laptop speaker, but user confirmation is still pending.

## Remaining work

- Complete firmware-side Wi-Fi failure classification and repeated-capture testing.
- Run the continuous Wi-Fi correctness and resource gates.
- Confirm speech fidelity and physical disconnect recovery.
- Finish the final end-to-end probe report.

The broader execution plan is in [AUDIO_PROBE_PLAN.md](AUDIO_PROBE_PLAN.md), and chronological results remain in [WORK_NOTES.md](WORK_NOTES.md).
