#!/usr/bin/env python3
"""合成番茄钟的提示音：一段钟声，后面接上中文语音。

输出 24 kHz、16-bit、双声道、小端序 PCM，与固件里其他语音资产一致。
钟声是加法合成的（基音加两个衰减更快的泛音），不依赖任何第三方音频素材。

用法：
    tools/make-pomodoro-audio.py focus <语音.pcm> <输出.pcm>
    tools/make-pomodoro-audio.py break <语音.pcm> <输出.pcm>

语音 PCM 由 macOS 的 `say -v Tingting` 生成，再用 ffmpeg 转成同样的格式，
见 firmware/main/assets/README.md。
"""

from __future__ import annotations

import math
import struct
import sys

RATE = 24000
# 峰值归一化到 -1 dBFS，与其他语音资产一致。
PEAK = 10 ** (-1 / 20)
# 钟声与语音之间留一小段安静。
GAP_SECONDS = 0.15

# (频率倍数, 增益, 衰减时间常数)。泛音衰减得比基音快，听起来才像钟而不像蜂鸣。
PARTIALS = ((1.0, 1.0, 0.9), (2.0, 0.35, 0.45), (3.01, 0.15, 0.3))

# 专注结束：下行的“叮—咚”，像放学铃，让人松一口气。
FOCUS_CHIME = ((659.25, 0.0), (523.25, 0.5))
FOCUS_LENGTH = 2.4
# 休息结束：上行三音，比“叮咚”亮一点，提醒该回来了。
BREAK_CHIME = ((523.25, 0.0), (659.25, 0.22), (783.99, 0.44))
BREAK_LENGTH = 2.0


def render_chime(notes: tuple[tuple[float, float], ...], length: float) -> bytes:
    samples = [0.0] * int(RATE * length)
    for frequency, start in notes:
        first = int(start * RATE)
        for index in range(first, len(samples)):
            t = (index - first) / RATE
            value = 0.0
            for ratio, gain, tau in PARTIALS:
                value += gain * math.sin(2 * math.pi * frequency * ratio * t) * math.exp(-t / tau)
            # 5 ms 的起音，避免起始处的咔嗒声。
            if t < 0.005:
                value *= t / 0.005
            samples[index] += value
    scale = PEAK / max(abs(sample) for sample in samples)
    output = bytearray()
    for sample in samples:
        level = int(max(-1.0, min(1.0, sample * scale)) * 32767)
        output += struct.pack("<hh", level, level)
    return bytes(output)


def main(argv: list[str]) -> int:
    if len(argv) != 4 or argv[1] not in ("focus", "break"):
        print(__doc__, file=sys.stderr)
        return 2
    kind, voice_path, output_path = argv[1:]
    if kind == "focus":
        chime = render_chime(FOCUS_CHIME, FOCUS_LENGTH)
    else:
        chime = render_chime(BREAK_CHIME, BREAK_LENGTH)
    with open(voice_path, "rb") as voice_file:
        voice = voice_file.read()
    gap = bytes(int(GAP_SECONDS * RATE) * 4)
    with open(output_path, "wb") as output_file:
        output_file.write(chime + gap + voice)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
