#!/usr/bin/env python3
"""Turn a look sheet into the firmware's look format (firmware-rs/core/src/look.rs).

Usage:
    tools/make-look.py <sheet.png> <output.bin>

The sheet is four 48 × 64 cells side by side (192 × 64): normal, eyes closed, happy, sad. A pixel is
transparent when its alpha is below 128 or it is pure magenta (#FF00FF); everything else is reduced
to 15 colors shared by the four frames, the 16th being transparent. Needs Pillow.
"""

from __future__ import annotations

import struct
import sys

from PIL import Image

WIDTH, HEIGHT, FRAMES, COLORS = 48, 64, 4, 16


def rgb565(red: int, green: int, blue: int) -> int:
    return (red >> 3) << 11 | (green >> 2) << 5 | (blue >> 3)


def build(sheet: Image.Image) -> bytes:
    if sheet.size != (WIDTH * FRAMES, HEIGHT):
        raise ValueError(f"the sheet must be {WIDTH * FRAMES} x {HEIGHT}, four cells side by side; got {sheet.size}")
    rgba = sheet.convert("RGBA")
    opaque = [
        (x, y)
        for y in range(HEIGHT)
        for x in range(WIDTH * FRAMES)
        if (pixel := rgba.getpixel((x, y)))[3] >= 128 and pixel[:3] != (255, 0, 255)
    ]
    if not opaque:
        raise ValueError("the sheet is empty")
    # Quantize only the opaque pixels, so transparency doesn't take one of the 15 colors.
    strip = Image.new("RGB", (len(opaque), 1))
    strip.putdata([rgba.getpixel(point)[:3] for point in opaque])
    quantized = strip.quantize(colors=COLORS - 1, method=Image.Quantize.MEDIANCUT)
    palette = quantized.getpalette()[: (COLORS - 1) * 3]
    indices = [[0] * (WIDTH * FRAMES) for _ in range(HEIGHT)]
    flattened = quantized.get_flattened_data() if hasattr(quantized, "get_flattened_data") else quantized.getdata()
    for (x, y), index in zip(opaque, flattened):
        indices[y][x] = index + 1

    out = bytearray(b"LOOK" + bytes([WIDTH, HEIGHT, FRAMES, 0]))
    colors = [0] + [rgb565(*palette[i * 3 : i * 3 + 3]) for i in range(len(palette) // 3)]
    colors += [0] * (COLORS - len(colors))
    out += struct.pack(f"<{COLORS}H", *colors)
    for frame in range(FRAMES):
        for y in range(HEIGHT):
            row = indices[y][frame * WIDTH : (frame + 1) * WIDTH]
            out += bytes(row[x] | row[x + 1] << 4 for x in range(0, WIDTH, 2))
    return bytes(out)


def main(argv: list[str]) -> None:
    if len(argv) != 3:
        sys.exit(__doc__)
    look = build(Image.open(argv[1]))
    with open(argv[2], "wb") as output:
        output.write(look)
    print(f"{argv[2]}: {len(look)} bytes")


if __name__ == "__main__":
    main(sys.argv)
