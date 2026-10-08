#!/usr/bin/env python3
"""独占面包板串口，录两秒到本地 WAV。先退出 Vibe Buddy App。

用法：python capture-mic.py /dev/cu.usbmodemXXXX output.wav
需要 pyserial；不上传录音、不启用持续监听。
"""
from __future__ import annotations

import array
import base64
import json
import math
import sys
import time
import wave
from pathlib import Path

import serial


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    destination = Path(sys.argv[2])
    with serial.Serial(sys.argv[1], 115200, timeout=0.5) as port:
        port.dtr = False
        port.rts = False
        # 打开原生 USB 可能触发复位，先等启动，再核对固件板型。
        time.sleep(3)
        port.reset_input_buffer()
        port.write(b'{"version":1,"event":"device.hello"}\n')
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if port.readline().strip() == b"BOARD goouuu-s3-spi":
                break
        else:
            raise RuntimeError("没有收到面包板身份，不开始录音")
        print("准备录音：请对麦克风说一句话（2 秒）。", flush=True)
        for remaining in (3, 2, 1):
            print(remaining, flush=True)
            time.sleep(1)
        port.write(b'{"version":1,"event":"device.mic.capture"}\n')
        data = bytearray()
        started = ended = False
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            line = port.readline().decode("ascii", errors="replace").strip()
            if line == "MIC RECORDING 2":
                print("正在录音…", flush=True)
            elif line == "MIC BEGIN 16000 32000":
                started = True
            elif line.startswith("MIC DATA ") and started:
                _, _, offset, encoded = line.split()
                if int(offset) != len(data):
                    raise RuntimeError("录音传输缺块，不保存不完整文件")
                data.extend(base64.b64decode(encoded, validate=True))
                if len(data) > 64000:
                    raise RuntimeError("录音超过预期长度")
            elif line == "MIC END":
                ended = True
                break
            elif line.startswith("MIC ERROR"):
                raise RuntimeError(line)
        if not ended or len(data) != 64000:
            raise RuntimeError(f"录音不完整：{len(data)}/64000 字节")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(destination), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(16000)
        output.writeframes(data)
    samples = array.array("h", data)
    if sys.byteorder != "little":
        samples.byteswap()
    mean = sum(samples) / len(samples)
    rms = math.sqrt(sum((sample - mean) ** 2 for sample in samples) / len(samples))
    print(json.dumps({"file": str(destination), "samples": len(samples),
                      "min": min(samples), "max": max(samples),
                      "ac_rms": round(rms, 2),
                      "clipped": sum(abs(s) >= 32767 for s in samples)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
