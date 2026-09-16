#!/usr/bin/env python3
"""把一个音色目录下的五句 PCM 打成语音包。

用法：
    tools/make_voice_pack.py <音色目录> <音色 id> <输出.bin>

音色目录里要有 input_required / done / failed / focus_done / break_done 五个
.pcm（24 kHz、16-bit、双声道），顺序固定。字节布局见 firmware/main/agent_voice_pack.h。
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
        raise ValueError("音色 id 需为 1 到 31 个 ASCII 字符")
    if len(clips) != 5 or any(len(clip) == 0 for clip in clips):
        raise ValueError("需要五段非空的 PCM")
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
    print(f"{output}: {len(pack)} 字节，音色 {voice_id}")


if __name__ == "__main__":
    main(sys.argv)
