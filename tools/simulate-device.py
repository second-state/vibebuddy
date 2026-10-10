#!/usr/bin/env python3
"""Open a pseudo-terminal, attach the firmware-core simulator to it, and print a device path usable as a serial port.

Without a box, use it to get serial tools such as tools/firmware-smoke.py working first:
    python3 tools/simulate-device.py            # prints /dev/ttysNNN; Ctrl-C to stop
    uv run --with pyserial python tools/firmware-smoke.py /dev/ttysNNN --no-ask
    python3 tools/simulate-device.py --tcp 7340 # also listens on 127.0.0.1:7340, as a box on Wi-Fi
"""

import os
import pty
import subprocess
import sys
import termios
import tty
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

subprocess.run(["cargo", "build", "--quiet", "-p", "vibebuddy-firmware-core", "--example", "simulator"], cwd=REPO, check=True)
master, slave = pty.openpty()
# Raw mode: no echo and no line editing, like a real serial port.
tty.setraw(slave)
attributes = termios.tcgetattr(slave)
attributes[3] &= ~termios.ECHO
termios.tcsetattr(slave, termios.TCSANOW, attributes)
print(os.ttyname(slave), flush=True)
process = subprocess.Popen([str(REPO / "target/debug/examples/simulator"), *sys.argv[1:]], stdin=master, stdout=master)
try:
    sys.exit(process.wait())
except KeyboardInterrupt:
    process.terminate()
