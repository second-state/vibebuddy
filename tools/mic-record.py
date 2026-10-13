#!/usr/bin/env python3
"""Microphone bring-up check: send `device.mic.record` to a box, collect the recording, write a WAV and say
whether each I2S slot carries a live microphone.

Needs exclusive use of the serial port, so quit the VibeBuddy app first. The port must be named: the box may
share a Mac with other ESP32-S3 devices, which enumerate the same way (LESSONS.md).

Usage:
    uv run --with pyserial python tools/mic-record.py --port /dev/cu.usbmodemXXXX [--ms 3000] [--gain-db 30] [--out mic.wav]

Talk, clap or tap near the box while it records. The firmware sends the samples back after recording,
about 11 s per recorded second (the UART is always written at 115200 baud, whichever port you use).
See docs/firmware-bringup.md, "Microphone", for what the output means.
"""

import argparse
import base64
import json
import math
import sys
import time
import wave
import zlib

import serial

FULL_SCALE = 32768


def dbfs(value):
    return -120.0 if value <= 0 else 20 * math.log10(value / FULL_SCALE)


def verdict(levels):
    """What one slot's 100 ms levels say about it."""
    rms = [level[0] for level in levels]
    peaks = [level[1] for level in levels]
    if max(peaks) == 0:
        return "SILENT (all zeros: nothing drives this slot, or wrong pin/slot)"
    if max(peaks) >= FULL_SCALE - 1 and sum(peak >= FULL_SCALE - 1 for peak in peaks) > len(peaks) // 2:
        return "SATURATED (pinned at full scale in most windows: gain too high or a floating pin)"
    spread = dbfs(max(rms)) - dbfs(max(min(rms), 1))
    if spread < 6:
        return f"FLAT (level barely moves, {spread:.1f} dB: noise floor or DC only; make some noise and retry)"
    return f"LIVE (level moves {spread:.1f} dB between quiet and loud windows)"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--port", required=True, help="serial port of the box, e.g. /dev/cu.usbmodem8401")
    parser.add_argument("--ms", type=int, default=3000, help="recording length, 100 to 5000 ms (default 3000)")
    parser.add_argument("--gain-db", type=int, default=None, help="ES8311 microphone gain, 0 to 42 dB in 6 dB steps (firmware default 30)")
    parser.add_argument("--out", default="mic.wav", help="WAV file to write (default mic.wav)")
    args = parser.parse_args()

    fields = {"version": 1, "event": "device.mic.record", "ms": args.ms}
    if args.gain_db is not None:
        fields["gain_db"] = args.gain_db
    line = json.dumps(fields, separators=(",", ":")).encode() + b"\n"

    port = serial.Serial(args.port, 115200, timeout=0.5)
    port.reset_input_buffer()
    # Short enough for the UART bridge in one piece (it corrupts writes over about 200 bytes).
    port.write(line)

    rate = channels = bits = size = None
    chunks = []
    levels = []
    end = None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        text = port.readline().decode("utf-8", "replace").strip()
        if not text.startswith("MIC "):
            continue
        words = text.split(" ")
        kind = words[1]
        if kind == "DATA":
            if int(words[2]) != len(chunks):
                print(f"missing data: got line {words[2]}, expected {len(chunks)}", file=sys.stderr)
                return 1
            chunks.append(base64.b64decode(words[3]))
            deadline = time.monotonic() + 5
            continue
        print(text)
        if kind in ("UNAVAILABLE", "BUSY", "ERROR"):
            return 1
        if kind in ("RECORDING", "CAPTURED"):
            deadline = time.monotonic() + 10
        elif kind == "LEVEL":
            levels.append(((int(words[4]), int(words[5])), (int(words[7]), int(words[8]))))
        elif kind == "BEGIN":
            rate, channels, bits, size = (int(word) for word in words[2:6])
            # About 11 s per recorded second over the UART, plus margin.
            deadline = time.monotonic() + 30 + size / 8000
        elif kind == "END":
            end = (int(words[2]), int(words[3]))
            break
    port.close()

    if end is None:
        print(f"no MIC END before the timeout ({len(chunks)} data lines received)", file=sys.stderr)
        return 1
    samples = b"".join(chunks)
    if len(samples) != end[0] or zlib.crc32(samples) != end[1]:
        print(f"recording damaged on the way: {len(samples)} bytes, CRC {zlib.crc32(samples)}; box sent {end[0]} bytes, CRC {end[1]}", file=sys.stderr)
        return 1

    with wave.open(args.out, "wb") as output:
        output.setnchannels(channels)
        output.setsampwidth(bits // 8)
        output.setframerate(rate)
        output.writeframes(samples)
    print(f"wrote {args.out}: {len(samples) // (channels * bits // 8) / rate:.2f} s, {rate} Hz, {channels} ch, {bits} bit")

    if levels:
        print("\n  time   left rms/peak (dBFS)   right rms/peak (dBFS)")
        for index, (left, right) in enumerate(levels):
            print(f"{index * 100:5d} ms   {dbfs(left[0]):6.1f} / {dbfs(left[1]):6.1f}       {dbfs(right[0]):6.1f} / {dbfs(right[1]):6.1f}")
        left = verdict([left for left, _ in levels])
        right = verdict([right for _, right in levels])
        print(f"\nleft slot:  {left}\nright slot: {right}")
        # The right slot is expected to carry the speaker's reference, silent unless the box is speaking.
        live = left.startswith("LIVE") or right.startswith("LIVE")
        print(f"\nmicrophone on this box: {'WORKS' if live else 'NOT SHOWN'} (listen to {args.out} to be sure)")
        return 0 if live else 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
