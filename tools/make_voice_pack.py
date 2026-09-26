#!/usr/bin/env python3
"""Pack the five PCM clips in a voice directory into a voice pack.

Usage:
    tools/make_voice_pack.py <voice dir> <voice id> <output.bin>

The voice directory must contain the five input_required / done / failed / focus_done / break_done
.pcm files (24 kHz, 16-bit, stereo), in that fixed order. Byte layout: see firmware/main/agent_voice_pack.h.
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

HEADER_BYTES = 256
CLIP_NAMES = ["input_required", "done", "failed", "focus_done", "break_done"]


def build(voice_id: str, clips: list[bytes]) -> bytes:
    encoded_id = voice_id.encode("ascii")
    if not encoded_id or len(encoded_id) > 31:
        raise ValueError("voice id must be 1 to 31 ASCII characters")
    if len(clips) != 5 or any(len(clip) == 0 for clip in clips):
        raise ValueError("need five non-empty PCM clips")
    payload = b"".join(clips)
    header = bytearray(HEADER_BYTES)
    header[0:4] = b"VBVP"
    struct.pack_into("<III", header, 4, 1, len(payload), zlib.crc32(payload))
    header[16 : 16 + len(encoded_id)] = encoded_id
    offset = HEADER_BYTES
    for index, clip in enumerate(clips):
        struct.pack_into("<I", header, 48 + index * 4, offset)
        struct.pack_into("<I", header, 68 + index * 4, len(clip))
        offset += len(clip)
    struct.pack_into("<I", header, 88, zlib.crc32(bytes(header[:88])))
    return bytes(header) + payload


def main(argv: list[str]) -> None:
    if len(argv) != 4:
        sys.exit(__doc__)
    voice_dir, voice_id, output = Path(argv[1]), argv[2], Path(argv[3])
    clips = [(voice_dir / f"{name}.pcm").read_bytes() for name in CLIP_NAMES]
    pack = build(voice_id, clips)
    output.write_bytes(pack)
    print(f"{output}: {len(pack)} bytes, voice {voice_id}")


if __name__ == "__main__":
    main(sys.argv)
