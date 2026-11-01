# XGGF speech recording and daemon transfer probe plan

## Objective

Prove that the XGGF can capture understandable speech and transfer it intact to the VibeBuddy daemon over USB and encrypted Wi-Fi, with bounded memory, explicit consent, visible recording state, and no default audio retention.

Do not change transport architecture based on an unclassified failure. Each gate must produce evidence before the next one starts.

## Current implementation

### Firmware

- 8 and 16 kHz I2S acquisition, selecting the left channel from 32-bit stereo slots.
- Mono signed 16-bit little-endian PCM.
- Numbered 20 ms frames with byte length, flags, and base64 payload.
- A bounded 32-frame ring, holding 0.64 seconds rather than a whole recording.
- Explicit start/stop, a 60-second maximum, visible `REC`, and raw-slot/ring-overrun counters.
- Recording-priority scheduling on the responsive main loop; the unstable dedicated-task experiment was removed.
- No firmware-side audio retention.

### Daemon

- Token-gated start, stop, delete, WAV, and statistics endpoints.
- In-memory WAV reconstruction without audio-content logging or default disk retention.
- Validation of base64, frame size, sequence, duration, and session state.
- Accounting for gaps, duplicates, reordering, disconnects, and timing normalization.
- Six ingestion unit tests, plus firmware fidelity, ring, continuous-frame, and overrun tests.

### Bandwidth

At 8 kHz, mono signed 16-bit PCM is 128 kbit/s, not 32 kbit/s. At 16 kHz it is 256 kbit/s. Base64 framing raises these to approximately 171 and 342 kbit/s before encryption and link overhead. Measure and report raw PCM and encoded throughput separately.

## Verified evidence

- USB 8 kHz speech: exact five seconds and 40,000 samples, with no clipping, loss, duplication, reordering, or overruns.
- USB 16 kHz speech: exact five seconds and 80,000 samples, with the same clean result.
- USB continuous acquisition: exact 30 seconds and 240,000 samples at 8 kHz, with zero reported loss or overruns.
- Individual encrypted Wi-Fi 16 kHz five-second captures completed with exact reconstructed duration, but required timing normalization.
- Returned WAVs were played through the laptop speaker. The user has not yet confirmed subjective intelligibility.
- Physical tests used the repository-root `DEVICE.lock` and released it after flashing and data collection.

## Unresolved work

1. Wi-Fi 16 kHz completion is intermittent. Partial writes, link loss, and missing `END` are hypotheses, not established causes. No 30-second Wi-Fi stress run has passed.
2. `last_error` diagnostics are incomplete. Failure paths must preserve exact terminal reasons, and per-link send failures, partial writes, and disconnects need counters.
3. Actual channel correctness and understandable speech still need explicit listening confirmation.
4. Cancellation and malformed input have unit coverage, but the complete physical matrix and intentional disconnect recovery are unfinished.
5. Long-run heap, stack, panel timing, and Wi-Fi interference measurements are incomplete.
6. Full project regression checks, final cleanup verification, and the comprehensive probe report remain.

## Gate 1: finish failure instrumentation without changing behavior

- Complete `last_error` and return terminal reasons through the statistics endpoint.
- Record peer state, encryption failures, partial-write counts, queue-full counts, and disconnect reasons.
- Preserve live frame counters on failure without logging PCM.
- Keep the Wi-Fi-only diagnostic runner for fast, repeatable failure collection.

Exit condition: every failed Wi-Fi run is classified as acquisition, encryption, TCP write, link disconnect, missing frame, missing `END`, or daemon rejection.

## Gate 2: establish a repeatable Wi-Fi baseline

- Run ten consecutive five-second 16 kHz Wi-Fi recordings.
- Compare results with the stable USB baseline.
- Change only the measured failure class:
  - Partial writes: bounded retry queue or backpressure.
  - Encryption size: enforce a protocol buffer contract.
  - Acquisition starvation: adjust scheduling.
  - Missing `END`: fix completion reliability.
  - Daemon rejection: fix parser or session state.

Exit condition: 10/10 recordings complete with no unexplained loss, duplication, reordering, or disconnects.

## Gate 3: continuous streaming and resource checks

- Run a 30-second Wi-Fi 16 kHz capture, then 60 seconds if stable.
- Record heap/stack headroom, panel transfer time, raw slots, overruns, timing normalization, arrival intervals, and end lag.
- Report observed arrival lag and jitter. Do not claim synchronized one-way latency without synchronized clocks.
- Check panel and Wi-Fi interference explicitly.

Exit condition: bounded memory, no sustained backlog, no unexplained acquisition loss, and documented timing behavior.

## Gate 4: fidelity and recovery

- Replay known speech at 8/16 kHz over USB and Wi-Fi for listening confirmation.
- Verify channel, signedness, bit alignment, sample rate, exact duration, and clipping from returned WAVs.
- Exercise cancellation, intentional disconnect/restart, malformed and oversized frames, and unauthorized API access.

Exit condition: speech is understandable end to end, and every failure path rejects or discards data safely.

## Gate 5: regression and comprehensive report

- Run all firmware host tests, daemon tests, desktop checks, localization checks, and physical USB/Wi-Fi regressions.
- Write the comprehensive Markdown report covering protocol, bandwidth, measurements, failures and fixes, limitations, privacy, and the production boundary.
- Confirm temporary pairings are removed and `DEVICE.lock` is released.

## Production boundary

Production audio capture must require explicit start/stop, use a bounded duration, show a visible recording indication, retain no audio by default, and define USB/Wi-Fi priority and disconnect behavior. Do not proceed beyond the probe until a spoken recording reaches the daemon intact and is confirmed understandable end to end.
