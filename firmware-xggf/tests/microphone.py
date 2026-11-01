"""Probe the XGGF microphone, optionally comparing a laptop tone with a baseline."""
import subprocess
import sys
import time
import serial


def probe(link, tone=None):
    link.reset_input_buffer()
    link.write(b'{"version":1,"event":"device.mic"}\n')
    link.flush()
    player = subprocess.Popen(["aplay", "-q", tone]) if tone else None
    values = []
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            line = link.readline().decode("utf-8", "replace").strip()
            if "Guru Meditation" in line or line.startswith("MIC ERROR"):
                raise AssertionError(line)
            if not line.startswith("MIC "):
                continue
            print(line, flush=True)
            if line == "MIC DONE":
                break
            fields = dict(item.split("=", 1) for item in line[4:].split())
            values.append({key: int(value) for key, value in fields.items()})
        else:
            raise AssertionError("microphone probe did not finish")
    finally:
        if player:
            try:
                player.wait(timeout=6)
            except subprocess.TimeoutExpired:
                player.terminate()
                player.wait()
    if len(values) < 5:
        raise AssertionError(f"only {len(values)} microphone reports arrived")
    return values


with serial.Serial(sys.argv[1], 115200, timeout=0.3) as link:
    baseline = probe(link)
    if len(sys.argv) > 2:
        stimulated = probe(link, sys.argv[2])
        baseline_level = max(value["mean"] for value in baseline)
        stimulated_level = max(value["mean"] for value in stimulated)
        if stimulated_level <= max(1000, baseline_level * 2):
            raise AssertionError(
                f"tone did not produce a clear response: baseline {baseline_level}, tone {stimulated_level}"
            )
        print(
            f"PASS: laptop tone increased microphone amplitude from {baseline_level} "
            f"to {stimulated_level}"
        )
    else:
        print(f"PASS: bounded microphone probe returned {len(baseline)} amplitude reports")
