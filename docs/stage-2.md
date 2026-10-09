# Stage 2 — `vibebuddyd` acceptance record

Accepted: 2026-09-14 (Asia/Singapore)

## Conclusion

Stage 2 **has passed on-device acceptance**. A local HTTP request on the Mac reaches `vibebuddy-fw` via `vibebuddyd`, `SerialTransport`, ESP32-S3 native USB Serial/JTAG and the VibeBuddy Protocol; after a real USB unplug and replug, the daemon automatically rediscovered and reconnected to the device, and events sent after the reconnect were delivered as well.

## Implementation boundaries

- `POST /v1/events` listens only on `127.0.0.1:7331` by default.
- `vibebuddy-protocol` handles version, non-empty event, unknown extension fields, NDJSON encoding and the 1024-byte limit.
- `SerialTransport` by default discovers the single device with `VID:PID 303A:1001` and does not hard-code `/dev/cu.usbmodem8401`.
- `VIBEBUDDY_SERIAL_PORT` can specify the serial port explicitly; `VIBEBUDDY_USB_SERIAL` can pick the target among several devices of the same model.
- The send queue holds 64 items. HTTP `202 Accepted` only means enqueued; when the queue is full or the worker has stopped, it returns `503 Service Unavailable`.
- After a serial read or write failure, the current frame whose flush has not been confirmed is kept, and the device is rediscovered and reconnected every 500 ms.

This stage did not install a macOS background service, and did not implement the `beacon` CLI, LCD, audio or buttons.

## Automated checks

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Result: 6 tests passed, no Clippy warnings. The tests cover Hello without `id`, preservation of unknown fields, unsupported version, oversized messages, HTTP enqueueing with NDJSON framing, and USB serial format normalization.

## HTTP → USB → ESP32 on-device link

The daemon automatically discovered and opened:

```text
serial port connected port=/dev/cu.usbmodem8401
```

Sent:

```json
{"version":1,"event":"task.start","id":"stage2-live","title":"Stage 2 HTTP"}
```

HTTP actually returned `202 Accepted`. The ESP32 then actually returned:

```text
EVENT task.start
TITLE Stage 2 HTTP
```

A request with `version: 2` actually returned `400 Bad Request` and never entered the serial queue.

## Real disconnect and reconnect

With the same `vibebuddyd` process kept running, the user unplugged `USB-SLAVE`, and the log recorded:

```text
serial read failed, reconnecting port=/dev/cu.usbmodem8401 error=Device not configured (os error 6)
```

After plugging it back in, without restarting the daemon:

```text
serial port connected port=/dev/cu.usbmodem8401
```

Then sent:

```json
{"version":1,"event":"task.done","id":"stage2-reconnect","title":"Reconnect verified"}
```

The device actually returned:

```text
EVENT task.done
TITLE Reconnect verified
```

What this stage proves, then, is that the same daemon process resumes transport after a real USB disconnect and re-enumeration, not a process restart or a local simulation.
