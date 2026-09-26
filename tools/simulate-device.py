#!/usr/bin/env python3
"""开一个伪终端，把 firmware-core 的模拟器挂上去，打印出可以当串口用的设备路径。

没有盒子时用它先把 tools/firmware-smoke.py 这类串口工具跑通：
    python3 tools/simulate-device.py            # 打印 /dev/ttysNNN，Ctrl-C 结束
    uv run --with pyserial python tools/firmware-smoke.py /dev/ttysNNN --no-ask
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
# 原始模式：不回显、不做行编辑，和真串口一样。
tty.setraw(slave)
attributes = termios.tcgetattr(slave)
attributes[3] &= ~termios.ECHO
termios.tcsetattr(slave, termios.TCSANOW, attributes)
print(os.ttyname(slave), flush=True)
process = subprocess.Popen([str(REPO / "target/debug/examples/simulator")], stdin=master, stdout=master)
try:
    sys.exit(process.wait())
except KeyboardInterrupt:
    process.terminate()
