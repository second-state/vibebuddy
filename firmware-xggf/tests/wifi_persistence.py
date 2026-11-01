"""Verify the configured network and device identity survive a reset."""
import json
import sys
import time
import serial

with serial.Serial(sys.argv[1], 115200, timeout=.3) as link:
    def hello():
        link.write(b'{"version":1,"event":"device.hello"}\n'); link.flush()
    def capture(seconds):
        end = time.monotonic() + seconds
        found = {}
        next_hello = 0
        while time.monotonic() < end:
            if time.monotonic() >= next_hello:
                hello(); next_hello = time.monotonic() + 3
            line = link.readline().decode("utf-8", "replace").strip()
            for key in ("BOX KEY", "WIFI NETWORK", "WIFI ADDRESS"):
                if line.startswith(key + " "): found[key] = line[len(key) + 1:]
            if len(found) == 3: return found
        raise AssertionError("identity, network or Wi-Fi address missing")
    before = capture(25)
    link.rts = True; time.sleep(.2); link.rts = False
    time.sleep(2)
    after = capture(35)
    assert before["BOX KEY"] == after["BOX KEY"]
    assert before["WIFI NETWORK"] == after["WIFI NETWORK"]
    print("PASS: device identity and configured Wi-Fi survived reset")
