#!/usr/bin/env python3
"""Open a pseudo-terminal, attach the firmware-core simulator to it, and print a device path usable as a serial port.

Without a box, use it to get serial tools such as tools/firmware-smoke.py working first:
    python3 tools/simulate-device.py            # prints /dev/ttysNNN; Ctrl-C to stop
    uv run --with pyserial python tools/firmware-smoke.py /dev/ttysNNN --no-ask
    python3 tools/simulate-device.py --tcp 7340 # also listens on 127.0.0.1:7340, as a box on Wi-Fi
    python3 tools/simulate-device.py --control /tmp/vb-control.sock  # test-only buttons, clock and reboot
"""

import argparse
import os
import pty
import subprocess
import sys
import tty
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--tcp", type=int)
    parser.add_argument("--relay")
    parser.add_argument("--build", help="override the local device build label (integration tests use 'simulator')")
    parser.add_argument("--control", help="test-only Unix socket for buttons, clock and reboot")
    args = parser.parse_args()
    if args.control and Path(args.control).exists():
        parser.error("the control socket path already exists; choose an unused path")
    subprocess.run(["cargo", "build", "--locked", "--quiet", "-p", "vibebuddy-firmware-core", "--example", "simulator"],
                   cwd=REPO, check=True)
    master, slave = pty.openpty()
    tty.setraw(slave)
    options = []
    for name in ("tcp", "relay", "control", "build"):
        if (value := getattr(args, name)) is not None:
            options.extend(["--" + name, str(value)])
    process = None
    try:
        process = subprocess.Popen([str(REPO / "target/debug/examples/simulator"), *options], stdin=master, stdout=master)
        print(os.ttyname(slave), flush=True)
        return process.wait()
    except KeyboardInterrupt:
        return 130
    finally:
        if process and process.poll() is None:
            process.terminate()
            process.wait(timeout=3)
        os.close(master)
        os.close(slave)
        if args.control:
            Path(args.control).unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
