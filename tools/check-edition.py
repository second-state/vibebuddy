#!/usr/bin/env python3
"""Check that a box reports the company edition it was flashed with (ADR-0026).

Usage: tools/check-edition.py <port> <edition id>

Sends device.hello and waits for `EDITION <id>` and `VOICES <id>`: the box has the edition, and wears its
company Character. Exits 0 when both come, 1 otherwise. Needs pyserial.
"""

from __future__ import annotations

import sys
import time

import serial

HELLO = b'{"version":1,"event":"device.hello"}\n'


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        sys.exit(__doc__)
    port, edition_id = argv[1], argv[2]
    wanted = {f"EDITION {edition_id}", f"VOICES {edition_id}"}
    seen: set[str] = set()
    deadline = time.monotonic() + 15
    next_hello = 0.0
    with serial.Serial(port, 115200, timeout=0.2) as device:
        while time.monotonic() < deadline and not wanted <= seen:
            if time.monotonic() >= next_hello:
                device.write(HELLO)
                device.flush()
                next_hello = time.monotonic() + 3
            line = device.readline().decode("utf-8", errors="replace").strip()
            if line.startswith(("EDITION ", "VOICES ")):
                print(line)
                seen.add(line)
    if wanted <= seen:
        print(f"PASS edition {edition_id}")
        return 0
    print(f"FAIL expected {sorted(wanted - seen)}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
