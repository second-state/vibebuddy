#!/usr/bin/env python3
"""Build a Character pack from a directory of synthesized lines.

Usage:
    tools/character_pack.py <character id> <audio dir> <output.bin>

The audio directory holds one subdirectory per occasion (input_required, done, first_done, ...),
each with that pool's lines as 16 kHz, 16-bit, mono, little-endian PCM files, taken in name order.
Occasions without a directory have no pool. Byte layout: see firmware-rs/core/src/character_pack.rs
and docs/characters.md; the ADPCM encoder matches the decoder in firmware-rs/core/src/adpcm.rs.
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

HEADER_BYTES = 1024
SAMPLE_RATE = 16000
CODEC_IMA_ADPCM = 1
MAX_LINES = (1020 - 128) // 8
OCCASIONS = [
    "input_required",
    "done",
    "failed",
    "focus_done",
    "break_done",
    "first_done",
    "milestone",
    "late_night_done",
    "late_night_input",
    "greeting_morning",
    "greeting_afternoon",
    "greeting_evening",
]

STEPS = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
    143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166,
    1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
    8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
]
INDEX_CHANGE = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8]


def encode_adpcm(samples: list[int]) -> bytes:
    """IMA ADPCM, starting from predictor 0 and step index 0, low nibble first."""
    predictor, index = 0, 0
    out = bytearray()
    for start in range(0, len(samples), 2):
        byte = 0
        for half, sample in enumerate(samples[start : start + 2]):
            step = STEPS[index]
            diff = sample - predictor
            nibble = 0
            if diff < 0:
                nibble, diff = 8, -diff
            if diff >= step:
                nibble |= 4
                diff -= step
            if diff >= step >> 1:
                nibble |= 2
                diff -= step >> 1
            if diff >= step >> 2:
                nibble |= 1
            # Track exactly what the decoder will reconstruct.
            delta = step >> 3
            if nibble & 4:
                delta += step
            if nibble & 2:
                delta += step >> 1
            if nibble & 1:
                delta += step >> 2
            predictor = predictor - delta if nibble & 8 else predictor + delta
            predictor = max(-32768, min(32767, predictor))
            index = max(0, min(88, index + INDEX_CHANGE[nibble]))
            byte |= nibble << (half * 4)
        out.append(byte)
    return bytes(out)


def build(character_id: str, pools: dict[str, list[list[int]]]) -> bytes:
    encoded_id = character_id.encode("ascii")
    if not encoded_id or len(encoded_id) > 31:
        raise ValueError("character id must be 1 to 31 ASCII characters")
    unknown = set(pools) - set(OCCASIONS)
    if unknown:
        raise ValueError(f"unknown occasions: {sorted(unknown)}")
    header = bytearray(HEADER_BYTES)
    payload = bytearray()
    line = 0
    for slot, occasion in enumerate(OCCASIONS):
        lines = pools.get(occasion, [])
        struct.pack_into("<HH", header, 56 + slot * 4, line if lines else 0, len(lines))
        for samples in lines:
            if not samples:
                raise ValueError(f"an empty line in {occasion}")
            if line == MAX_LINES:
                raise ValueError(f"more than {MAX_LINES} lines")
            struct.pack_into("<II", header, 128 + line * 8, HEADER_BYTES + len(payload), len(samples))
            payload += encode_adpcm(samples)
            line += 1
    header[0:4] = b"VBCP"
    struct.pack_into("<III", header, 4, 1, len(payload), zlib.crc32(payload))
    header[16 : 16 + len(encoded_id)] = encoded_id
    struct.pack_into("<IBBH", header, 48, SAMPLE_RATE, CODEC_IMA_ADPCM, len(OCCASIONS), line)
    struct.pack_into("<I", header, 1020, zlib.crc32(bytes(header[:1020])))
    return bytes(header) + bytes(payload)


def read_pcm(path: Path) -> list[int]:
    data = path.read_bytes()
    return list(struct.unpack(f"<{len(data) // 2}h", data[: len(data) // 2 * 2]))


def main(argv: list[str]) -> None:
    if len(argv) != 4:
        sys.exit(__doc__)
    character_id, audio_dir, output = argv[1], Path(argv[2]), Path(argv[3])
    pools = {
        occasion: [read_pcm(path) for path in sorted((audio_dir / occasion).glob("*.pcm"))]
        for occasion in OCCASIONS
        if (audio_dir / occasion).is_dir()
    }
    pack = build(character_id, {occasion: lines for occasion, lines in pools.items() if lines})
    output.write_bytes(pack)
    count = sum(len(lines) for lines in pools.values())
    print(f"{output}: {len(pack)} bytes, {count} lines, character {character_id}")


if __name__ == "__main__":
    main(sys.argv)
