#!/usr/bin/env python3
"""Build the firmware's Chinese glyph table, firmware-rs/core/fonts/cjk-12px.bin.

The box draws ASCII with its own 5x7 font (canvas.rs); everything else it can draw comes from this table:
the 3,500 characters of level one of the Table of General Standard Chinese Characters (通用规范汉字表 一级,
2013), the CJK punctuation and the full-width forms, rasterized by Fusion Pixel Font 12 px (SIL OFL 1.1).

Both inputs are pinned by URL and SHA-256, so the output is the same on every machine:
  - Fusion Pixel Font, 12 px monospaced, BDF, its zh_Hans glyph shapes. Fusion is Ark Pixel Font with the
    characters Ark Pixel hasn't drawn yet at 12 px filled in from Cubic 11 (both OFL).
  - Unicode's Unihan database, whose kTGH field gives each character's number in the 2013 table;
    numbers 1 to 3500 are level one.

Downloads are cached in ~/.cache/vibebuddy-fonts. Pass --fusion and --unihan to use local copies.

Output layout (little-endian), read by firmware-rs/core/src/font.rs:
  u16 count
  u16 codepoint[count], ascending (all in the Basic Multilingual Plane)
  18-byte bitmap[count]: the 12x12 cell, 12 rows of 12 pixels, row-major, most significant bit first.
    The cell's baseline is under row 9 (ascent 10, descent 2).

The font's license files (OFL.txt, LICENSES/) are written next to the table.

Usage: tools/make-cjk-font.py [--fusion ZIP] [--unihan ZIP] [--output PATH]
"""
import argparse
import hashlib
import io
import struct
import sys
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "firmware-rs/core/fonts/cjk-12px.bin"
CACHE = Path.home() / ".cache/vibebuddy-fonts"

FUSION_VERSION = "2026.09.25"
FUSION_URL = ("https://github.com/TakWolf/fusion-pixel-font/releases/download/"
              f"{FUSION_VERSION}/fusion-pixel-font-12px-monospaced-bdf-v{FUSION_VERSION}.zip")
FUSION_SHA256 = "d75f5262f108757edb0f47ee8e3d2dfdfbecfd94558faf5ffb0c06dc5866fb3b"
FUSION_MEMBER = "fusion-pixel-12px-monospaced-zh_hans.bdf"
UNIHAN_URL = "https://www.unicode.org/Public/16.0.0/ucd/Unihan.zip"
UNIHAN_SHA256 = "b8f000df69de7828d21326a2ffea462b04bc7560022989f7cc704f10521ef3e0"
UNIHAN_MEMBER = "Unihan_OtherMappings.txt"

LEVEL_ONE = 3500
CELL = 12
ASCENT = 10
GLYPH_BYTES = CELL * CELL // 8

# Besides the level-one characters: what Chinese text needs around them. Ranges are inclusive; a codepoint
# the font has no full-width glyph for is skipped (the box draws a box for it).
EXTRA_RANGES = [
    (0x3000, 0x303F),  # CJK symbols and punctuation: 、。〈〉《》「」『』【】〔〕〜 and the ideographic space
    (0xFF01, 0xFF5E),  # full-width ASCII forms: ，！？：；（）０-９ Ａ-Ｚ
    (0xFFE0, 0xFFE6),  # full-width ￠￡￢￣￤￥￦
]
EXTRA_CHARACTERS = "‘’“”…—―‖‥※·→←↑↓"


def fetch(url, sha256, override):
    if override:
        data = Path(override).read_bytes()
    else:
        cached = CACHE / url.rsplit("/", 1)[1]
        if not cached.exists():
            CACHE.mkdir(parents=True, exist_ok=True)
            print(f"downloading {url}", file=sys.stderr)
            with urllib.request.urlopen(url) as response:
                cached.write_bytes(response.read())
        data = cached.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest != sha256:
        sys.exit(f"{override or url}: SHA-256 {digest}, expected {sha256}")
    return zipfile.ZipFile(io.BytesIO(data))


def level_one(unihan):
    characters = []
    for line in unihan.read(UNIHAN_MEMBER).decode().splitlines():
        if not line.startswith("U+"):
            continue
        codepoint, field, value = line.split("\t")
        if field == "kTGH" and int(value.split(":")[1]) <= LEVEL_ONE:
            characters.append(int(codepoint[2:], 16))
    if len(characters) != LEVEL_ONE:
        sys.exit(f"Unihan lists {len(characters)} level-one characters, expected {LEVEL_ONE}")
    return characters


def parse_bdf(text):
    """codepoint -> (advance, 12x12 cell as rows of 0/1, or None when the glyph reaches outside the cell)."""
    lines = iter(text.splitlines())
    glyphs = {}
    for line in lines:
        if line.startswith("FONT_ASCENT ") and int(line.split()[1]) != ASCENT:
            sys.exit(f"expected a font ascent of {ASCENT}: {line}")
        if not line.startswith("STARTCHAR"):
            continue
        codepoint = advance = box = None
        for line in lines:
            key, _, value = line.partition(" ")
            if key == "ENCODING":
                codepoint = int(value)
            elif key == "DWIDTH":
                advance = int(value.split()[0])
            elif key == "BBX":
                box = [int(number) for number in value.split()]
            elif key == "BITMAP":
                break
        width, height, x_offset, y_offset = box
        stride = (width + 7) // 8 * 8
        cell = [[0] * CELL for _ in range(CELL)]
        for row in range(height):
            bits = int(next(lines), 16)
            for column in range(width):
                if bits >> (stride - 1 - column) & 1:
                    # BDF rows run down from the box's top, which is y_offset + height above the baseline.
                    cell_row = ASCENT - (y_offset + height) + row
                    cell_column = x_offset + column
                    if cell is not None and 0 <= cell_row < CELL and 0 <= cell_column < CELL:
                        cell[cell_row][cell_column] = 1
                    else:
                        cell = None
        if next(lines) != "ENDCHAR":
            sys.exit(f"U+{codepoint:04X}: malformed BDF")
        glyphs[codepoint] = (advance, cell)
    return glyphs


def pack(cell):
    bits = 0
    for row in cell:
        for pixel in row:
            bits = bits << 1 | pixel
    return bits.to_bytes(GLYPH_BYTES, "big")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--fusion", help="local copy of the Fusion Pixel Font BDF zip")
    parser.add_argument("--unihan", help="local copy of Unihan.zip")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    arguments = parser.parse_args()

    fusion = fetch(FUSION_URL, FUSION_SHA256, arguments.fusion)
    glyphs = parse_bdf(fusion.read(FUSION_MEMBER).decode())
    hanzi = level_one(fetch(UNIHAN_URL, UNIHAN_SHA256, arguments.unihan))
    unusable = [codepoint for codepoint in hanzi
                if codepoint not in glyphs or glyphs[codepoint][0] != CELL or glyphs[codepoint][1] is None]
    if unusable:
        sys.exit("no full-width 12x12 glyph for level-one characters: " + "".join(map(chr, unusable)))

    extras = [codepoint for first, last in EXTRA_RANGES for codepoint in range(first, last + 1)]
    extras += [ord(character) for character in EXTRA_CHARACTERS]
    # Only full-width glyphs inside the cell: every glyph in the table advances 12 px. The rest are left to
    # the fallback box.
    extras = [codepoint for codepoint in extras
              if codepoint in glyphs and glyphs[codepoint][0] == CELL and glyphs[codepoint][1] is not None]
    codepoints = sorted(set(hanzi) | set(extras))
    if codepoints[-1] > 0xFFFF:
        sys.exit("the table's index holds 16-bit codepoints only")

    table = bytearray(struct.pack("<H", len(codepoints)))
    table += b"".join(struct.pack("<H", codepoint) for codepoint in codepoints)
    table += b"".join(pack(glyphs[codepoint][1]) for codepoint in codepoints)
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_bytes(table)
    # The table is a derivative of the font, so it carries the font's licenses: Fusion Pixel's own and those
    # of the fonts it is built from.
    for member in fusion.namelist():
        if member == "OFL.txt" or member.startswith("LICENSES/") and not member.endswith("/"):
            license_file = arguments.output.parent / member
            license_file.parent.mkdir(parents=True, exist_ok=True)
            license_file.write_bytes(fusion.read(member))
    skipped = sorted(set(ord(character) for character in EXTRA_CHARACTERS) - set(codepoints))
    print(f"{len(codepoints)} glyphs ({len(hanzi)} hanzi, {len(codepoints) - len(hanzi)} other), "
          f"{len(table)} bytes -> {arguments.output}")
    if skipped:
        print("not full-width in the font, left to the fallback box: " + "".join(map(chr, skipped)))


if __name__ == "__main__":
    main()
