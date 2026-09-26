#!/usr/bin/env python3
"""新固件上机后的冒烟检查：串口能判断的自动判断，要人看、要人听的逐条提示。

用法（先退出 Vibe Buddy App，让出串口）：
    uv run --with pyserial python tools/firmware-smoke.py /dev/cu.usbmodemXXXX

它依次检查：握手与构建标识、长行串口完整性、四种状态的显示与播报、音量、
眨眼确认、截图（存成 PPM，可以直接打开看），最后把音量还原、状态回到空闲。
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
        # 和 daemon 一样按 128 字节分段写：经 UART 桥时一次写太多会被桥吞错。
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
        """收到所有 wanted 行（前缀匹配）就返回收到的全部行；超时返回 None。"""
        pending = list(wanted)
        seen = []
        for line in self.lines(seconds):
            seen.append(line)
            pending = [item for item in pending if not line.startswith(item)]
            if not pending:
                return seen
        print(f"    没等到: {pending}；收到: {seen[-8:]}")
        return None


results = []


def check(name, ok, detail=""):
    results.append((name, ok))
    print(f"{'PASS' if ok else 'FAIL'}  {name}{('  ' + detail) if detail else ''}")


def ask(prompt):
    answer = input(f"  人工确认：{prompt} [y/n] ").strip().lower()
    return answer.startswith("y")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("port")
    parser.add_argument("--no-ask", action="store_true", help="跳过要人看、要人听的确认")
    parser.add_argument("--shot", default="firmware-smoke.ppm", help="截图存到这里")
    args = parser.parse_args()
    confirm = (lambda prompt: True) if args.no_ask else ask
    device = Device(args.port)

    print("== 握手")
    device.send("device.hello")
    seen = device.expect(["DISPLAY READY BUILD ", "MODE ", "VOICES ", "VOLUME "])
    check("hello 回报构建号、模式、音色、音量", seen is not None)
    volume = None
    if seen:
        build = next(line for line in seen if line.startswith("DISPLAY READY BUILD "))[len("DISPLAY READY BUILD "):]
        volume = next(line for line in seen if line.startswith("VOLUME "))[len("VOLUME "):]
        expected = (REPO / "firmware-rs/device/build/build.txt")
        if expected.exists():
            check("盒子上的构建标识就是刚打的这一版", build == expected.read_text().strip(), build)
        print(f"    音色 {next(line for line in seen if line.startswith('VOICES '))[7:]}，音量 {volume}")

    print("== 串口完整性（一行接近协议上限）")
    data = "".join(chr(33 + (index * 7) % 94) for index in range(900)).replace('"', "a").replace("\\", "b")
    device.send("device.echo", data=data)
    seen = device.expect(['{"version":1,"event":"echo"'])
    ok = False
    if seen:
        reply = json.loads(next(line for line in seen if line.startswith('{"version":1,"event":"echo"')))
        ok = reply["length"] == len(data) and reply["crc"] == zlib.crc32(data.encode())
    check("900 字节的一行原样收到", ok)

    print("== 四种状态")
    tasks = [{"title": "CC:SMOKE TEST", "status": "running", "elapsed_s": 5, "project": "VIBE-BUDDY"}]
    device.send("task.start", title="CC:SMOKE TEST", tasks=tasks)
    check("工作中", device.expect(["EVENT task.start", "DISPLAY STATE WORKING"]) is not None)
    check("屏幕：橙色、左边一张任务卡、小灯灵脸上是 > 和跳动的点", confirm("屏幕是否如上"))

    tasks[0]["status"] = "input_required"
    device.send("agent.input_required", title="CC:SMOKE TEST", tasks=tasks)
    check("需要确认 + 播报", device.expect(["DISPLAY STATE INPUT REQUIRED", "AUDIO QUEUED INPUT_REQUIRED"]) is not None)
    check("听到「需要确认」那一句，画面转黄", confirm("听到了吗"))

    tasks[0]["status"] = "failed"
    device.send("task.error", title="CC:SMOKE TEST", tasks=tasks)
    check("失败 + 播报", device.expect(["DISPLAY STATE FAILED", "AUDIO QUEUED FAILED"]) is not None)

    tasks[0]["status"] = "done"
    device.send("task.done", title="CC:SMOKE TEST", tasks=tasks, stats=["3 DONE", "1 ASKS"])
    check("完成 + 播报", device.expect(["DISPLAY STATE DONE", "AUDIO QUEUED DONE"]) is not None)
    check("听到「完成」、画面转绿、5 秒后自己回到空闲", confirm("是否如此"))

    print("== 音量")
    device.send("device.volume", level=40, preview=True)
    seen = device.expect(["VOLUME 40", "AUDIO QUEUED DONE"])
    check("音量调到 40 并试听", seen is not None)
    check("试听明显比刚才轻", confirm("是否变轻"))
    if volume and volume.isdigit():
        device.send("device.volume", level=int(volume))
        check(f"音量还原为 {volume}", device.expect([f"VOLUME {volume}"]) is not None)

    print("== 眨眼确认")
    device.send("device.identify")
    check("IDENTIFY", device.expect(["IDENTIFY"]) is not None)
    check("背光快闪了约一秒", confirm("闪了吗"))

    print("== 截图")
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
    check("截图完整（76800 像素）", ok, f"背光 {backlight}")
    if ok:
        rgb = bytearray()
        for pixel in pixels:
            rgb += bytes([((pixel >> 11) & 0x1F) * 255 // 31, ((pixel >> 5) & 0x3F) * 255 // 63, (pixel & 0x1F) * 255 // 31])
        Path(args.shot).write_bytes(b"P6\n320 240\n255\n" + bytes(rgb))
        print(f"    截图存到 {args.shot}，和屏幕上的样子对一下")

    device.send("agent.idle")
    device.expect(["DISPLAY STATE READY"])

    failed = [name for name, ok in results if not ok]
    print(f"\n共 {len(results)} 项，失败 {len(failed)} 项")
    for name in failed:
        print(f"  - {name}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
