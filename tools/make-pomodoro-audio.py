#!/usr/bin/env python3
"""Synthesize the pomodoro cues: a chime followed by the spoken line.

Output is 24 kHz, 16-bit, stereo, little-endian PCM, matching the firmware's other voice assets.
The chime is additive synthesis (a fundamental plus two faster-decaying overtones), with no third-party audio.

Usage:
    tools/make-pomodoro-audio.py focus <voice.pcm> <output.pcm>
    tools/make-pomodoro-audio.py break <voice.pcm> <output.pcm>

The voice PCM comes from tools/make-voices.sh (neural TTS, then converted by ffmpeg to
the same format and normalized); see firmware/main/assets/README.md.
"""

from __future__ import annotations

import math
import struct
import sys

RATE = 24000
# Normalize the peak to -1 dBFS, matching the other voice assets.
PEAK = 10 ** (-1 / 20)
# Leave a short silence between the chime and the voice.
GAP_SECONDS = 0.15

# (frequency multiple, gain, decay time constant). Overtones decay faster than the fundamental, so it sounds like a bell, not a buzzer.
PARTIALS = ((1.0, 1.0, 0.9), (2.0, 0.35, 0.45), (3.01, 0.15, 0.3))

# Focus done: a falling “ding-dong”, like a school bell, so you can relax.
FOCUS_CHIME = ((659.25, 0.0), (523.25, 0.5))
FOCUS_LENGTH = 2.4
# Break done: three rising notes, a little brighter than the “ding-dong”, a reminder to come back.
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
            # A 5 ms attack avoids a click at the start.
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
