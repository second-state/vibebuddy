#!/usr/bin/env python3
"""Smoke check for new firmware on a real box: what the serial line can decide is checked automatically; what
needs eyes or ears is asked one item at a time.

Usage (quit the VibeBuddy app first so it releases the serial port):
    uv run --with pyserial python tools/firmware-smoke.py /dev/cu.usbmodemXXXX

It checks, in order: the handshake and build ID, serial integrity on a long line, how the four states are shown
and announced, volume, the identify blink, and a screenshot (saved as a PPM you can open directly). Finally it
restores the volume and returns the box to idle.
"""

import argparse
import json
import sys
import time
import zlib
from pathlib import Path

import serial

REPO = Path(__file__).resolve().parent.parent


class Device:
    def __init__(self, port):
        self.serial = serial.Serial(port, 115200, timeout=0.1)
        self.buffer = b""

    def send(self, event, **fields):
        message = {"version": 1, "event": event, **fields}
        line = json.dumps(message, separators=(",", ":"), ensure_ascii=False).encode() + b"\n"
        # Write in 128-byte pieces like the daemon does: the UART bridge corrupts larger writes.
        for start in range(0, len(line), 128):
            self.serial.write(line[start:start + 128])
            self.serial.flush()
            time.sleep(0.012)

    def lines(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.buffer += self.serial.read(4096)
            while b"\n" in self.buffer:
                raw, self.buffer = self.buffer.split(b"\n", 1)
                yield raw.decode("utf-8", errors="replace").rstrip("\r")

    def expect(self, wanted, seconds=3.0):
        """Return every line received once all `wanted` lines (prefix match) have arrived; None on timeout."""
        pending = list(wanted)
        seen = []
        for line in self.lines(seconds):
            seen.append(line)
            pending = [item for item in pending if not line.startswith(item)]
            if not pending:
                return seen
        print(f"    never saw: {pending}; received: {seen[-8:]}")
        return None


results = []


def check(name, ok, detail=""):
    """`ok` is True, False, or None for a human check that was skipped with --no-ask."""
    results.append((name, ok))
    verdict = "SKIP" if ok is None else "PASS" if ok else "FAIL"
    print(f"{verdict}  {name}{('  ' + detail) if detail else ''}")


def ask(prompt):
    answer = input(f"  check by hand: {prompt} [y/n] ").strip().lower()
    return answer.startswith("y")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("port")
    parser.add_argument("--no-ask", action="store_true", help="skip the checks that need someone to look or listen (reported as SKIP)")
    parser.add_argument("--shot", default="firmware-smoke.ppm", help="where to save the screenshot")
    args = parser.parse_args()
    confirm = (lambda prompt: None) if args.no_ask else ask
    device = Device(args.port)

    print("== Handshake")
    device.send("device.hello")
    seen = device.expect(["DISPLAY READY BUILD ", "MODE ", "VOICES ", "VOLUME "])
    check("hello reports build, mode, voice and volume", seen is not None)
    volume = None
    if seen:
        build = next(line for line in seen if line.startswith("DISPLAY READY BUILD "))[len("DISPLAY READY BUILD "):]
        volume = next(line for line in seen if line.startswith("VOLUME "))[len("VOLUME "):]
        expected = (REPO / "firmware-rs/device/build/build.txt")
        if expected.exists():
            check("the box runs the build just made", build == expected.read_text().strip(), build)
        print(f"    voice {next(line for line in seen if line.startswith('VOICES '))[7:]}, volume {volume}")

    print("== Serial integrity (a line close to the protocol limit)")
    data = "".join(chr(33 + (index * 7) % 94) for index in range(900)).replace('"', "a").replace("\\", "b")
    device.send("device.echo", data=data)
    seen = device.expect(['{"version":1,"event":"echo"'])
    ok = False
    if seen:
        reply = json.loads(next(line for line in seen if line.startswith('{"version":1,"event":"echo"')))
        ok = reply["length"] == len(data) and reply["crc"] == zlib.crc32(data.encode())
    check("a 900-byte line arrives intact", ok)

    print("== The four states")
    tasks = [{"title": "CC:SMOKE TEST", "status": "running", "elapsed_s": 5, "project": "VIBE-BUDDY"}]
    device.send("task.start", title="CC:SMOKE TEST", tasks=tasks)
    check("working", device.expect(["EVENT task.start", "DISPLAY STATE WORKING"]) is not None)
    check("screen: orange, one task card on the left, > and bouncing dots on the face", confirm("does the screen look like that"))

    tasks[0]["status"] = "input_required"
    device.send("agent.input_required", title="CC:SMOKE TEST", tasks=tasks)
    check("input required + announcement", device.expect(["DISPLAY STATE INPUT REQUIRED", "AUDIO QUEUED INPUT_REQUIRED"]) is not None)
    check("heard the input-required line, screen turned yellow", confirm("did you hear it"))

    tasks[0]["status"] = "failed"
    device.send("task.error", title="CC:SMOKE TEST", tasks=tasks)
    check("failed + announcement", device.expect(["DISPLAY STATE FAILED", "AUDIO QUEUED FAILED"]) is not None)

    tasks[0]["status"] = "done"
    device.send("task.done", title="CC:SMOKE TEST", tasks=tasks, stats=["3 DONE", "1 ASKS"])
    check("done + announcement", device.expect(["DISPLAY STATE DONE", "AUDIO QUEUED DONE"]) is not None)
    check("heard the done line, screen turned green, back to idle after 5 s", confirm("did it"))

    print("== Volume")
    device.send("device.volume", level=40, preview=True)
    seen = device.expect(["VOLUME 40", "AUDIO QUEUED DONE"])
    check("volume set to 40 with a preview", seen is not None)
    check("the preview was clearly quieter", confirm("was it quieter"))
    if volume and volume.isdigit():
        device.send("device.volume", level=int(volume))
        check(f"volume restored to {volume}", device.expect([f"VOLUME {volume}"]) is not None)

    print("== Identify")
    device.send("device.identify")
    check("IDENTIFY", device.expect(["IDENTIFY"]) is not None)
    check("the backlight blinked for about a second", confirm("did it blink"))

    print("== Screenshot")
    device.send("device.screenshot")
    pixels = []
    backlight = None
    for line in device.lines(20):
        if line.startswith("SHOT BEGIN"):
            backlight = line.rsplit(" ", 1)[-1]
        elif line == "SHOT END":
            break
        elif line.startswith("SHOT "):
            for run in line.split(" ")[1:]:
                color, count = run.split(":")
                pixels.extend([int(color, 16)] * int(count))
    ok = len(pixels) == 320 * 240
    check("complete screenshot (76800 pixels)", ok, f"backlight {backlight}")
    if ok:
        rgb = bytearray()
        for pixel in pixels:
            rgb += bytes([((pixel >> 11) & 0x1F) * 255 // 31, ((pixel >> 5) & 0x3F) * 255 // 63, (pixel & 0x1F) * 255 // 31])
        Path(args.shot).write_bytes(b"P6\n320 240\n255\n" + bytes(rgb))
        print(f"    saved to {args.shot}; compare it with the screen")

    device.send("agent.idle")
    device.expect(["DISPLAY STATE READY"])

    failed = [name for name, ok in results if ok is False]
    skipped = sum(1 for _, ok in results if ok is None)
    print(f"\n{len(results)} checks, {len(failed)} failed, {skipped} skipped")
    for name in failed:
        print(f"  - {name}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
