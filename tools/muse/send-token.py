#!/usr/bin/env python3
"""Send a Muse SDK token to a box running Muse, the way Muse's web flasher does (web-flasher/sdk-token.js).

Muse stores the token in its NVS, answers "@sdk_token ok" and restarts. Get a token at gadgets.muse.ai >
Account > SDK tokens. It is yours: don't share it or put it in a box you sell.

Usage: send-token.py <port> <token>
"""

import re
import sys
import time

import serial

ANSWER = re.compile(r"@sdk_token (ok|error \S+)")


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    port, token = sys.argv[1], sys.argv[2]
    if not re.fullmatch(r"mgst_[A-Za-z0-9_-]{43}", token):
        sys.exit("that is not an SDK token: copy it again from gadgets.muse.ai")
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            link = serial.Serial(port, 115200, timeout=0.2)
        except serial.SerialException:
            time.sleep(1)  # the port comes back after the reset
            continue
        with link:
            sent = 0.0
            text = ""
            while time.monotonic() < deadline:
                if time.monotonic() - sent > 3:
                    link.write(f"\n>sdk_token={token}\n".encode())
                    sent = time.monotonic()
                try:
                    text += link.read(256).decode(errors="replace")
                except serial.SerialException:
                    break  # Muse restarted; open the port again
                answer = ANSWER.search(text)
                if answer:
                    if answer.group(1) == "ok":
                        print("token sent; Muse restarts with it")
                        return
                    sys.exit(f"Muse refused the token: {answer.group(1)}")
                text = text[-512:]
    sys.exit("no answer from Muse; is the box running it?")


if __name__ == "__main__":
    main()
