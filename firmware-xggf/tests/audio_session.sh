#!/bin/bash
# One DEVICE.lock-held hardware session for the XGGF audio gates.
#
# Prints its scope before touching the device, keeps a TCP-state log while the probe runs, and is
# bounded by the caller's `timeout` so a cancelled or hung session cannot keep recording. Run from
# the repository root:
#
#   flock DEVICE.lock timeout -k 10 420 firmware-xggf/tests/audio_session.sh PORT CONNECTION OUTDIR
#
# PORT is the device's USB serial port, CONNECTION an active NetworkManager connection whose
# credentials stay outside this repository, OUTDIR where results and logs are kept. RECORDINGS
# overrides the repeat count (default 10). The speech stimulus plays aloud once per recording.
set -u
PORT="${1:?serial port}"
CONNECTION="${2:?NetworkManager connection}"
OUT="${3:?output dir}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RECORDINGS="${RECORDINGS:-10}"
cd "$ROOT"
mkdir -p "$OUT"

echo "=== XGGF hardware session ================================"
echo "Scope: 1) list device pairings over USB, unpair probe leftovers only;"
echo "       2) run $RECORDINGS five-second 16 kHz Wi-Fi recordings (speech"
echo "          stimulus will play aloud once per recording)."
echo "Expected duration: about 3-4 minutes. Hard stop via the caller's timeout."
echo "==========================================================="

MONITOR_PID=""
cleanup() { [ -n "$MONITOR_PID" ] && kill "$MONITOR_PID" 2>/dev/null; }
trap cleanup EXIT INT TERM

( while :; do date +%s; ss -tinp '( dport = :7340 or sport = :7340 )' 2>/dev/null; echo ---; sleep 1; done \
    > "$OUT/ss.log" 2>&1 ) &
MONITOR_PID=$!

status=0

# Pairing hygiene: keep the real computer's key, drop probe leftovers.
python3 - "$PORT" <<'PYEOF' || status=$?
import json, os, serial, sys, time
port = sys.argv[1]
real = open(os.path.expanduser("~/.config/vibebuddy/identity")).read().strip()
with serial.Serial(port, 115200, timeout=0.5) as link:
    def send(line, seconds=2.0):
        link.write((line + "\n").encode()); link.flush()
        out, end = [], time.monotonic() + seconds
        while time.monotonic() < end:
            text = link.readline().decode(errors="replace").strip()
            if text: out.append(text)
        return out
    keys = []
    for text in send(json.dumps({"version": 1, "event": "device.pairs"})):
        if text.startswith("PAIR "):
            keys.append(text.split()[1])
    print(f"device reports {len(keys)} paired key(s)")
    for key in (key for key in dict.fromkeys(keys) if key != real):
        send(json.dumps({"version": 1, "event": "device.unpair", "key": key}), seconds=1.0)
        print("sent unpair for a probe leftover")
    after = [t for t in send(json.dumps({"version": 1, "event": "device.pairs"})) if t.startswith("PAIRS ")]
    print("after:", after[-1] if after else "count unknown")
    print(f"real computer key was paired: {real in keys}")
PYEOF

# The repeat sequence: ten five-second recordings, no 30-second tail (--quick).
python3 firmware-xggf/tests/audio_probe.py target/debug/vibebuddyd "$PORT" "$CONNECTION" \
    "$OUT/probe" --wifi-only --quick --repeat="$RECORDINGS" 2>&1 | tee "$OUT/probe-output.log" || status=$?

exit $status
