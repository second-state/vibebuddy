# LIFE-09: The status stream decides whether the daemon is alive

Category: App lifecycle and daemon supervision · Platform observed: Mac app (`app/`)

## Where it lives

- `app/Sources/VibeBuddy/DaemonClient.swift` (`stream`)
- `app/Sources/VibeBuddy/AppModel.swift` (`followStatus`, `apply`)

Daemon API: `GET /v1/status/stream (SSE)`; `GET /v1/resident`

## Initial state (input)

- The app is running.

## Trigger

The stream connects, delivers snapshots, stalls or drops.

## Workflow (Mac app)

1. The app opens `/v1/status/stream` with a 45 s inactivity timeout; the daemon's SSE keepalives keep a quiet stream alive.
2. Each `data:` line is decoded as a status snapshot; a line that doesn't decode is skipped.
3. The first snapshot after the daemon was down also asks `GET /v1/resident` whether the daemon runs the resident session, since only its environment at start decides (CONV-01).
4. Each snapshot: status replaced, the last notify toggle remembered, daemon marked alive, menu refreshed, foreign-firmware state refreshed, firmware notification checked (NOTIF-02), Sparkle's schedule set from `updates.enabled`, and a pending voice written once the box has reported a build (GEN-01).
5. A non-200 answer, a dropped stream or 45 s without data marks the daemon not alive and shows the daemon-down menu; the app retries every second.

## Final state (output)

- Alive while snapshots or keepalives arrive; down within about 45 s of a stall, at once on a drop.

## Check on Linux

- [ ] Does the Linux app use the same rule: alive on snapshots, down on a drop or a stall, retrying by itself?
- [ ] Known difference: Linux also bounds the response headers to 10 s; both use a 45 s inactivity timeout.

## Linux result

- Status: not checked
- Checked on: (date, commit, distribution and desktop)
- Evidence: (test, log, screenshot or code reference)
- Notes:
