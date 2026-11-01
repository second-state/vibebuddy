# XGGF speech recording and daemon transfer probe — comprehensive report

Status as of 2026-11-01: **all five gates of [AUDIO_PROBE_PLAN.md](AUDIO_PROBE_PLAN.md) have passed.** The XGGF captures understandable human speech over its I2S microphone and transfers it intact to the VibeBuddy daemon over both USB and the encrypted Wi-Fi link, with bounded memory, explicit consent, a visible recording state, and no default audio retention.

## Controls

- Every flash and physical capture held the repository-root `DEVICE.lock` with `flock` for the complete operation. Bounded sessions run through `tests/audio_session.sh`; the recovery matrix through `tests/recovery_matrix.py`, both wrapped in `timeout`.
- Each probe run starts an isolated daemon (private `XDG_CONFIG_HOME`/`XDG_STATE_HOME`, `VIBEBUDDY_BIND=127.0.0.1:17331`, a random `VIBEBUDDY_AUDIO_PROBE_TOKEN`), removes its temporary pairing afterward, and never prints network credentials. The build environment is ESP-IDF 5.5.4 with `IDF_TOOLS_PATH=~/.espressif-vibe`.
- Audio payload lines never enter any log: the daemon suppresses `AUDIO FRAME` at info level, and the firmware diagnostics are PCM-free.

## Protocol

Recording is daemon-initiated: `POST /v1/device/audio/start {"sample_rate": 8000|16000, "duration_ms": 1..60000}` sends `device.audio.start`; `device.audio.stop` ends early. The device replies with `AUDIO BEGIN rate=<r> samples=<r/50>`, numbered 20 ms frames `AUDIO FRAME <seq> <pcm_bytes> <flags> <base64>` (mono signed 16-bit little-endian PCM; flag 1 = final frame, 2 = ring-overrun discontinuity), and `AUDIO END <seq> <overruns> <raw_slots>`. Retrieval, cancellation, and statistics are `GET`/`DELETE /v1/device/audio` and `GET /v1/device/audio/stats`, token-gated and XGGF-only. The daemon reconstructs the WAV in memory, zero-fills gaps, counts loss/duplication/reordering, and normalizes timing within ±25 frames; a session that receives no audio line for ten seconds terminates with `device stopped sending the recording` (a powered-off peer sends no TCP reset).

## Bandwidth

Raw PCM is 128 kbit/s at 8 kHz and 256 kbit/s at 16 kHz; base64 framing raises this to approximately 171 and 342 kbit/s before encryption and link overhead. A sealed 16 kHz frame line measures 1,192 bytes.

## Verified evidence (final build)

| Check | Result |
|---|---|
| USB 8 kHz and 16 kHz five-second speech | Exact 40,000 / 80,000 samples; zero loss, duplication, reordering, overruns, clipping |
| USB 8 kHz thirty seconds | Exact 240,000 samples, clean |
| Wi-Fi ten-by-five-second 16 kHz, two independent sequences | All twenty exact; `timing_padding` 0-2 frames; `device_slots` 101-109% of real time; zero loss |
| Wi-Fi thirty seconds | Exact 480,000 samples; a 1.2 s TCP window stall ridden out inside the send path with no teardown |
| Wi-Fi sixty seconds | Exact 960,000 samples; worst arrival interval 228 ms |
| Listening confirmation | The user played the full set (USB and Wi-Fi speech, both long captures) and confirmed the recordings intelligible |
| Recovery matrix | Daemon restart, USB unplug, device power-cycle, and Wi-Fi loss mid-recording each end in a terminal reason with no partial WAV, and the next recording after the link returns is exact |

## Failure classes and their fixes

Each fix was flashed and re-measured on its own; the full chronology is in [WIFI_AUDIO_DIAGNOSTICS.md](WIFI_AUDIO_DIAGNOSTICS.md).

1. **Fatal short writes (the original Gate 1 finding).** Positive short writes closed the link. Fixed by resuming from the write offset.
2. **Send-timeout teardowns.** The 20 ms send wait treated routine TCP window closures as fatal on a link with ~150 ms RTT, causing reconnect storms (and the historical `recv_errno=113` failures, the same churn from the receive side). Fixed by a 4 s patience whose 40 ms slices drain the microphone, so waiting cannot starve acquisition.
3. **Acquisition starvation.** The I2S DMA queue held ~32 ms while loop iterations spent longer, silently losing samples: every prior Wi-Fi recording under-delivered to ~93%, masked by timing padding. Fixed with ~480 ms of DMA buffering (`device_slots` restored to 99-109%).
4. **Frame bleed during reconnect gaps.** Frames popped into dead sockets were lost. Fixed by requeuing each frame in a 96-slot ring until its line is confirmed sent, plus a bounded hold while a network-armed recording has no peer, and a 3 s delivery grace that finishes undeliverable recordings best-effort. A 160-slot ring was measured to starve the Wi-Fi stack's heap (`heap_min` 8.9 KB, association failing) and was backed out; 96 is the ceiling on this hardware.

## Resources (final build, Wi-Fi associated)

Free heap ~29.6 KB idle (minimum watermark 10.8 KB during full operation), main-stack headroom 5,244 bytes, panel transfer 15.86-15.87 ms unchanged with audio streaming — no measurable panel/Wi-Fi interference. The 30/60-second captures show no sustained backlog and no acquisition loss.

## Privacy and retention

Audio stays in memory on the daemon with no disk retention and no payload logging; the token gates every endpoint; the device retains nothing and shows `REC` while recording. Temporary pairings are removed by each harness when it finishes.

## Production boundary

Explicit start/stop, a 60 s duration cap, a visible recording indicator, no default retention, defined USB/Wi-Fi priority (USB preempts the network for 15 s), and verified disconnect behavior on every disruption path: all met and measured.

## Limitations

- A window stall longer than send patience plus ring depth still loses the affected frames, reported honestly as overruns with a discontinuity flag; deeper buffering is not affordable in RAM on the ESP32-C3.
- Evidence covers one unit on two networks (a home 2.4 GHz AP and a phone hotspot); other environments may stress the send patience differently.
- Relay-path audio is untested; the capability is USB and local Wi-Fi only.
- The recordings are only as good as the recovered microphone chain (16 kHz maximum); no echo cancellation or gain control exists.
- Subjective quality was confirmed by one listener on the tested set.
