#!/usr/bin/env python3
import argparse
import json
import time

import serial


EXPECTED = ["EVENT task.done", "TITLE Hello"]


def main() -> int:
    parser = argparse.ArgumentParser(description="Run the Stage 1 hardware hello check")
    parser.add_argument("port")
    parser.add_argument("--timeout", type=float, default=15.0)
    args = parser.parse_args()

    payload = {"version": 1, "event": "task.done", "title": "Hello"}
    deadline = time.monotonic() + args.timeout
    received: list[str] = []
    sent = False

    with serial.Serial(args.port, 115200, timeout=0.2) as device:
        while time.monotonic() < deadline:
            raw = device.readline()
            if raw:
                line = raw.decode("utf-8", errors="replace").strip()
                if line:
                    print(line)
                    received.append(line)
                if line.startswith("READY ") and not sent:
                    device.write((json.dumps(payload, separators=(",", ":")) + "\n").encode())
                    device.flush()
                    sent = True

            if not sent and time.monotonic() + 10.0 >= deadline:
                device.write((json.dumps(payload, separators=(",", ":")) + "\n").encode())
                device.flush()
                sent = True

            if all(expected in received for expected in EXPECTED):
                print("PASS Stage 1 Mac -> USB -> ESP32-S3 -> JSON parse")
                return 0

    print("FAIL expected output not observed")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
