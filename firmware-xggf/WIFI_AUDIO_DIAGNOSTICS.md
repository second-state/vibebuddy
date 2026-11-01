# XGGF Wi-Fi audio transport diagnostics

## Status

Gate 1 produced a measured firmware-side classification of the original failure: nonblocking TCP payload writes could complete short, and `send_line` treated those short writes as fatal. A bounded short-write retry was implemented and shown to recover some later short writes.

Gate 2 is **not met**. One same-session sequence passed 10/10 five-second 16 kHz captures, but an earlier equivalent sequence passed nine and then failed on the tenth with a fatal receive error (`recv_errno=113`). The cause and initiator of that receive failure remain unresolved. The isolated 10/10 run does not establish reliable behavior or erase the earlier failure.

Testing was stopped to prevent a loop. No 30-second Wi-Fi capture was run after the retry change, and USB audio was not rerun during this investigation.

## Controls and test setup

- The firmware was built with `IDF_TOOLS_PATH=/home/dabajabaza/.espressif-vibe` after sourcing `/home/dabajabaza/esp/esp-idf/export.sh` (ESP-IDF 5.5.4).
- Every flash and physical capture held the repository-root `DEVICE.lock` with `flock` for the complete operation and released it afterward.
- The current `target/debug/vibebuddyd` was copied to `/tmp/vibebuddyd-wifi-probe`; the probe used only that copy. Both files had SHA-256 `be0c8adff46c41129fec03a4344d6bf3a09366f176c279a08d5fb30371ae56c9` when copied. No daemon source was built or edited.
- The existing NetworkManager-backed harness used connection `ChinaNet-9zun`. Credentials were not printed or added to files.
- Each successful capture below contained exactly 80,000 samples and 160,000 PCM bytes at 16 kHz, exactly 5.0 seconds after daemon timing normalization. Unless stated otherwise, lost frames, duplicate frames, reordered frames, discontinuities, device overruns, and clipping were all zero.
- Raw PCM bandwidth at this rate is 256 kbit/s; base64 framing is approximately 342 kbit/s before encryption and link overhead. The observed sealed TCP payload length for an audio line was commonly 1,192 bytes.

## Code changes made during the investigation

### `firmware-xggf/main/wifi_link.c`

Added cumulative, PCM-free diagnostics for:

- Current encrypted-peer and TCP-client state.
- Accepted connections and peer-up/peer-down transitions.
- Encryption failures returned by `vb_lan_send`.
- Terminal send failures, short writes, `EAGAIN`/`EWOULDBLOCK`, and send timeouts.
- TCP disconnect count and last close class: protocol, input overflow, peer EOF, receive, send, encryption, or Wi-Fi.
- Last send part (payload or newline), requested length, last send result, errno, and last/maximum send wait.
- Last receive errno, Wi-Fi disconnect count, and last Wi-Fi reason.

`wifi_link_diagnose()` emits one `LINK DIAG` line over USB and, when available, the encrypted Wi-Fi link. It is emitted at link announcement and after `AUDIO END`. No PCM or credentials are included in this diagnostic.

The original fatal short-write path was then replaced with bounded retry/backpressure:

- Payload and newline writes resume from the correct byte offset after a positive short write.
- A short write or `EAGAIN`/`EWOULDBLOCK` waits for `POLLOUT` using `<sys/poll.h>` and retries.
- Each payload or newline write has a 20 ms total wait bound (`SEND_TIMEOUT_MS=20`).
- A persistent timeout or hard socket error still closes the link.
- No transmit queue was added.

### `firmware-xggf/main/wifi_link.h`

Declared `wifi_link_diagnose()`.

### `firmware-xggf/main/main.c`

After an `AUDIO END` report, called `wifi_link_diagnose()`. Audio acquisition, framing, ring buffering, and USB transfer logic were not changed.

### `firmware-xggf/tests/audio_probe.py`

- Added `--repeat=N` for 1–100 consecutive five-second Wi-Fi recordings on one daemon/Wi-Fi session.
- Printed only the latest sanitized `LINK DIAG` line after each successful recording.
- On failure, included sanitized audio-rejection, link-diagnostic, session-ending, and link-closed log lines rather than dumping the daemon log.

## Chronology and measurements

### 1. Instrumentation only: original short-write failure reproduced

The instrumented firmware built and flashed successfully before any transport behavior was changed.

| Run | Frames | Transport ms | Last lag ms | Max interval ms | Device slots | Timing padding |
|---|---:|---:|---:|---:|---:|---:|
| `instrumented-1` | 232 | 5,084 | 464 | 65 | 149,888 | 18 |
| `instrumented-2` | 232 | 5,079 | 459 | 84 | 149,376 | 18 |

The next run (`instrumented-3`) did not complete; `/v1/device/audio/stats` returned HTTP 409, `no completed recording statistics`.

The preserved firmware diagnostic read over USB afterward was:

```text
LINK DIAG peer=0 tcp=0 accepts=10 peer_up=10 peer_down=10 encrypt_fail=0 send_fail=6 partial=6 blocked=0 disconnects=10 wifi_disconnects=5 last_close=protocol send_part=1 send_len=1192 send_sent=392 send_errno=0 recv_errno=113 wifi_reason=5
```

Verified classification: six positive short payload writes had occurred, including 392 bytes accepted out of 1,192, with zero encryption failures. `last_close=protocol` reflected a later close associated with the diagnostic USB hello, not necessarily the original recording failure; the short-write fields were the useful preserved evidence. This justified changing only the short-write handling.

### 2. Twenty-ms retry: six independent sessions passed, then a stall

After building and flashing the bounded retry, independent `--wifi-only --quick` probe runs produced:

| Run | Frames | Transport ms | Last lag ms | Max interval ms | Device slots | Timing padding |
|---|---:|---:|---:|---:|---:|---:|
| `retry-1` | 231 | 5,037 | 437 | 72 | 148,992 | 19 |
| `retry-2` | 233 | 5,163 | 523 | 79 | 149,888 | 17 |
| `retry-3` | 233 | 5,140 | 500 | 104 | 150,016 | 17 |
| `retry-4` | 231 | 5,068 | 468 | 102 | 148,736 | 19 |
| `retry-5` | 234 | 5,106 | 446 | 96 | 151,040 | 16 |
| `retry-6` | 233 | 5,079 | 439 | 85 | 149,632 | 17 |

Run 7 stalled. Its live, incomplete statistics were 92 frames, 56 lost frames, a 711 ms maximum interval, and zero duplicates, reordered frames, discontinuities, and device overruns. Live `pcm_bytes` was still zero because no recording had completed.

The cumulative diagnostic afterward was:

```text
LINK DIAG peer=0 tcp=0 accepts=11 peer_up=11 peer_down=11 encrypt_fail=0 send_fail=4 partial=8 blocked=4 timeouts=3 disconnects=11 wifi_disconnects=11 last_close=protocol send_part=1 send_len=1192 send_sent=-1 send_errno=113 send_ms=0 max_send_ms=31 recv_errno=0 wifi_reason=5
```

These counters were cumulative across recordings and daemon teardowns. They proved that retried short writes, blocking, timeouts, and errno-113 send failures had occurred somewhere in the sequence, but did not assign a specific event to run 7. This ambiguity motivated the same-session repeat harness and per-recording diagnostic output.

### 3. First same-session ten-run sequence: nine passes, then receive failure

With one encrypted Wi-Fi session and `--repeat=10`, the first nine captures passed. Every per-recording diagnostic through run 9 reported zero send failures, partial writes, blocked sends, timeouts, and disconnects; `max_send_ms` was 2 ms.

| Run | Frames | Transport ms | Last lag ms | Max interval ms | Device slots | Timing padding |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 232 | 5,071 | 451 | 94 | 149,760 | 18 |
| 2 | 233 | 5,159 | 519 | 80 | 150,528 | 17 |
| 3 | 235 | 5,113 | 433 | 92 | 151,552 | 15 |
| 4 | 235 | 5,112 | 432 | 73 | 151,680 | 15 |
| 5 | 234 | 5,137 | 477 | 103 | 151,168 | 16 |
| 6 | 234 | 5,052 | 392 | 80 | 151,040 | 16 |
| 7 | 236 | 5,141 | 441 | 80 | 152,192 | 14 |
| 8 | 236 | 5,098 | 398 | 85 | 152,192 | 14 |
| 9 | 235 | 5,142 | 462 | 96 | 151,296 | 15 |

Run 10 stalled with 192 received frames, zero lost frames, a 154 ms maximum interval, and no duplicates, reordered frames, discontinuities, or device overruns. No `AUDIO END` completed.

The diagnostic obtained afterward was:

```text
LINK DIAG peer=0 tcp=0 accepts=1 peer_up=1 peer_down=1 encrypt_fail=0 send_fail=0 partial=0 blocked=0 timeouts=0 disconnects=1 wifi_disconnects=1 last_close=receive send_part=0 send_len=0 send_sent=0 send_errno=0 send_ms=0 max_send_ms=1 recv_errno=113 wifi_reason=5
```

This is the latest peer-reset/receive-failure finding. Verified facts are the fatal receive close, errno 113, and absence of any send, encryption, acquisition-overrun, or frame-loss counter explaining the stall. It was described as a peer reset during the investigation, but no sanitized daemon session-ending line was captured for this run. Therefore the peer initiator, the precise errno-113 cause in this environment, and whether the daemon or network triggered it remain unverified. No further transport change is justified by this evidence alone.

### 4. Later isolated same-session sequence: 10/10 passed

Without another firmware change, the next independent invocation of the same `--repeat=10` probe passed all ten recordings:

| Run | Frames | Transport ms | Last lag ms | Max interval ms | Device slots | Timing padding |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 233 | 5,089 | 449 | 60 | 150,400 | 17 |
| 2 | 234 | 5,138 | 478 | 74 | 151,040 | 16 |
| 3 | 235 | 5,235 | 555 | 127 | 151,296 | 15 |
| 4 | 234 | 5,106 | 446 | 62 | 150,912 | 16 |
| 5 | 234 | 5,175 | 515 | 84 | 151,040 | 16 |
| 6 | 235 | 5,101 | 421 | 76 | 151,168 | 15 |
| 7 | 235 | 5,182 | 502 | 108 | 151,296 | 15 |
| 8 | 228 | 5,175 | 635 | 138 | 146,944 | 22 |
| 9 | 233 | 5,086 | 446 | 121 | 150,144 | 17 |
| 10 | 235 | 5,102 | 422 | 78 | 151,552 | 15 |

The final diagnostic for this invocation was:

```text
LINK DIAG peer=1 tcp=1 accepts=2 peer_up=2 peer_down=1 encrypt_fail=0 send_fail=0 partial=1 blocked=0 timeouts=0 disconnects=1 wifi_disconnects=3 last_close=receive send_part=1 send_len=1192 send_sent=988 send_errno=0 send_ms=0 max_send_ms=28 recv_errno=113 wifi_reason=5
```

The device had not been reset between this and the preceding failed invocation. Consequently, `disconnects=1`, `peer_down=1`, `last_close=receive`, and `recv_errno=113` are historical evidence from that earlier failure, not a new disconnect in the 10/10 sequence. During run 8, one short payload write (988 of 1,192 bytes) was recovered without a terminal send failure, timeout, lost frame, or disconnect. This directly demonstrates that the bounded retry can handle the originally measured short-write class.

Results were saved under `/tmp/xggf-wifi-repeat-reset`; the earlier same-session failure sequence was under `/tmp/xggf-wifi-repeat-20ms`.

## Remaining issues

- The errno-113 receive failure remains intermittent and unexplained. A single subsequent 10/10 run is not sufficient to claim the Gate 2 reliability exit condition.
- The investigation did not establish whether that receive failure originates in the daemon, network, or firmware below the instrumented send path.
- The retry is bounded and queue-free, but no 30-second stress run or long-run resource measurement was performed after adding it.
- USB speech/capture had passed before this investigation, but no post-change USB regression was run.
- No daemon source or documentation was edited, and no commit was created.
