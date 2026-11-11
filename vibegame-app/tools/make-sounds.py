#!/usr/bin/env python3
"""Makes the VibeGame's sounds from the robot's voice (voices/jessica/), into assets/ as WAV.

    python3 vibegame-app/tools/make-sounds.py

The voice files are raw PCM: 24 kHz, 16-bit, stereo, little-endian. The VibeGame plays a line with
`tone_wav`, which takes 16-bit PCM of at most 2 s, so each line becomes 16 kHz mono and must fit in 2 s.
The WAVs are under CC BY-SA 4.0, as the voice is (LICENSE-ASSETS).
"""

import array
import pathlib
import sys
import wave

ROOT = pathlib.Path(__file__).resolve().parents[2]
LINES = ["input_required", "done", "failed"]
SOURCE_RATE = 24_000
RATE = 16_000
LIMIT_S = 2.0


def convert(pcm):
    stereo = array.array("h", pcm)
    if sys.byteorder != "little":
        stereo.byteswap()
    mono = [(stereo[i] + stereo[i + 1]) // 2 for i in range(0, len(stereo) - 1, 2)]
    out = array.array("h")
    for n in range(len(mono) * RATE // SOURCE_RATE):
        at = n * SOURCE_RATE / RATE
        i = int(at)
        nxt = mono[min(i + 1, len(mono) - 1)]
        out.append(round(mono[i] + (nxt - mono[i]) * (at - i)))
    return out


def main():
    for name in LINES:
        samples = convert((ROOT / "voices/jessica" / f"{name}.pcm").read_bytes())
        seconds = len(samples) / RATE
        if seconds > LIMIT_S:
            sys.exit(f"{name} is {seconds:.2f} s; tone_wav takes at most {LIMIT_S} s")
        if sys.byteorder != "little":
            samples.byteswap()
        target = ROOT / "vibegame-app/assets" / f"{name}.wav"
        with wave.open(str(target), "wb") as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(RATE)
            out.writeframes(samples.tobytes())
        print(f"{target.relative_to(ROOT)}: {seconds:.2f} s")


if __name__ == "__main__":
    main()
