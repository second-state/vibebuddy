#!/usr/bin/env python3
"""Draw the korekore box screens' type into firmware tables (ADR-0021, docs/design/korekore).

The design sets the box screens in DotGothic16 at 9 to 38 px. DotGothic16 is drawn on a 16 px grid: below 16 px its
strokes fall between pixels and come out as faint gray lines, and the letters run into each other. So each size is
drawn the way a pixel font draws well, whole pixels only (no anti-aliasing), at a font's own size or a whole
multiple of it:
  - 9 to 11 px: Fusion Pixel 10 px, proportional
  - 12 to 14 px: Fusion Pixel 12 px, proportional
  - 16 to 22 px: DotGothic16 at 16 px
  - 30 to 38 px: DotGothic16 at 16 px, doubled
Fusion Pixel is Ark Pixel with Cubic 11 filling in (SIL OFL 1.1, like the 12 px Chinese table, whose licenses are in
firmware-rs/core/fonts); DotGothic16 is the one the apps bundle (fonts/korekore). Every character keeps at least one
blank column after it. The "korekore" wordmark is M PLUS Rounded 1c ExtraBold, kept smooth: a rounded logo, not text.

The output is firmware-rs/core/src/korekore/glyphs.rs: each face's glyphs as 4-bit coverage (0 = none, 15 = full),
which the text faces only ever use as 0 or 15. Fusion Pixel is pinned by URL and SHA-256 and cached in
~/.cache/vibebuddy-fonts; DotGothic16 and the wordmark go through FreeType in Pillow, so a Pillow upgrade could move a
pixel: regenerate and commit, don't compare in CI.

Usage: tools/korekore-box-fonts.py
"""
import hashlib
import io
import urllib.request
import zipfile
from pathlib import Path

from PIL import ImageFont

ROOT = Path(__file__).resolve().parents[1]
FONTS = ROOT / "fonts/korekore"
OUT = ROOT / "firmware-rs/core/src/korekore/glyphs.rs"
CACHE = Path.home() / ".cache/vibebuddy-fonts"

FUSION_VERSION = "2026.09.25"
FUSION = {
    10: ("3d37465880abe927648b2c15c48f3d5442bcd801d78470ecbcffcfd3558ba6f0", "fusion-pixel-10px-proportional-latin.bdf"),
    12: ("7cae5ea83b46086f146002d194d46bbba70ec7264ac3a309219f5ad491626cef", "fusion-pixel-12px-proportional-latin.bdf"),
}

ASCII = "".join(chr(code) for code in range(0x20, 0x7F)) + "·›‹…×"
# The design's size, what draws it, and the characters it needs.
FACES = [
    (9, ("fusion", 10), ASCII),
    (10, ("fusion", 10), ASCII),
    (11, ("fusion", 10), ASCII),
    (12, ("fusion", 12), ASCII),
    (13, ("fusion", 12), ASCII),
    (14, ("fusion", 12), ASCII),
    (16, ("dot", 1), ASCII),
    (18, ("dot", 1), ASCII),
    (22, ("dot", 1), ASCII),
    (30, ("dot", 2), " 0123456789:ABCEFKORSU"),
    (34, ("dot", 2), "これ!"),
    (36, ("dot", 2), "YAT!"),
    (38, ("dot", 2), "0123456789:"),
]
WORDMARK = "korekore"
WORDMARK_SIZES = [12, 14, 40]


def fusion_bdf(size):
    sha, member = FUSION[size]
    name = f"fusion-pixel-font-{size}px-proportional-bdf-v{FUSION_VERSION}.zip"
    path = CACHE / name
    if not path.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        url = f"https://github.com/TakWolf/fusion-pixel-font/releases/download/{FUSION_VERSION}/{name}"
        path.write_bytes(urllib.request.urlopen(url).read())
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != sha:
        raise SystemExit(f"{name} doesn't match its pinned SHA-256")
    return zipfile.ZipFile(io.BytesIO(data)).read(member).decode()


def parse_bdf(text, wanted):
    """The wanted characters' pixels from a BDF: {character: (advance, left, top, rows of 0/1)}, and ascent, descent."""
    ascent = descent = 0
    glyphs = {}
    lines = iter(text.splitlines())
    for line in lines:
        if line.startswith("FONT_ASCENT "):
            ascent = int(line.split()[1])
        elif line.startswith("FONT_DESCENT "):
            descent = int(line.split()[1])
        elif line.startswith("STARTCHAR"):
            code = advance = None
            box = (0, 0, 0, 0)
            for line in lines:
                if line.startswith("ENCODING "):
                    code = int(line.split()[1])
                elif line.startswith("DWIDTH "):
                    advance = int(line.split()[1])
                elif line.startswith("BBX "):
                    box = tuple(int(value) for value in line.split()[1:5])
                elif line == "BITMAP":
                    width, height, x_offset, y_offset = box
                    rows = []
                    for _ in range(height):
                        bits = int(next(lines), 16)
                        span = (width + 7) // 8 * 8
                        rows.append([(bits >> (span - 1 - x)) & 1 for x in range(width)])
                    if code is not None and chr(code) in wanted:
                        glyphs[chr(code)] = (advance, x_offset, None, y_offset, rows)
                    break
    return {
        character: (advance, left, ascent - (y_offset + len(rows)), rows)
        for character, (advance, left, _, y_offset, rows) in glyphs.items()
    }, ascent, descent


def dot_gothic(characters, scale):
    """DotGothic16 at its own 16 px, whole pixels, scaled by a whole number."""
    font = ImageFont.truetype(str(FONTS / "DotGothic16-Regular.ttf"), 16)
    ascent, descent = font.getmetrics()
    glyphs = {}
    for character in characters:
        mask, (left, top) = font.getmask2(character, mode="1")
        width, height = mask.size
        rows = [[1 if mask.getpixel((x, y)) else 0 for x in range(width)] for y in range(height)]
        if scale > 1:
            rows = [[pixel for pixel in row for _ in range(scale)] for row in rows for _ in range(scale)]
        glyphs[character] = (round(font.getlength(character)) * scale, left * scale, top * scale, rows)
    return glyphs, ascent * scale, descent * scale


def trimmed(rows):
    """The rows with blank columns cut from either side, and how many came off the left."""
    width = max((len(row) for row in rows), default=0)
    used = [x for x in range(width) if any(row[x] for row in rows)]
    if not used:
        return [], 0
    first, last = used[0], used[-1]
    return [row[first : last + 1] for row in rows], first


def coverage(rows, width):
    values = [15 if pixel else 0 for row in rows for pixel in row]
    if len(values) % 2:
        values.append(0)
    return bytes(values[i] | values[i + 1] << 4 for i in range(0, len(values), 2))


def rust_bytes(data, indent):
    return "\n".join(indent + " ".join(f"0x{byte:02X}," for byte in data[i : i + 24]) for i in range(0, len(data), 24))


def face(size, source, characters):
    kind, value = source
    if kind == "fusion":
        glyphs, ascent, descent = parse_bdf(fusion_bdf(value), set(characters))
        what = f"Fusion Pixel {value} px"
    else:
        glyphs, ascent, descent = dot_gothic(characters, value)
        what = "DotGothic16 at 16 px" + (f", ×{value}" if value > 1 else "")
    rows_out = []
    alpha = b""
    for character in sorted(dict.fromkeys(characters), key=ord):
        if character not in glyphs:
            continue
        advance, left, top, rows = glyphs[character]
        rows, cut = trimmed(rows)
        left += cut
        width = len(rows[0]) if rows else 0
        # At least one blank column after every character, so neighbours never touch.
        advance = max(advance, left + width + 1) if width else advance
        rows_out.append(
            f"        Glyph {{ code: {ord(character)}, advance: {advance}, left: {left}, top: {top}, "
            f"width: {width}, height: {len(rows) if width else 0}, at: {len(alpha)} }},"
        )
        if width:
            alpha += coverage(rows, width)
    return (
        f"/// The design's {size} px, drawn by {what}.\n"
        f"pub static TYPE_{size}: Face = Face {{\n    size: {size},\n    ascent: {ascent},\n    descent: {descent},\n"
        f"    glyphs: &[\n" + "\n".join(rows_out) + f"\n    ],\n    alpha: &[\n{rust_bytes(alpha, '        ')}\n    ],\n}};\n"
    )


def wordmark(size):
    font = ImageFont.truetype(str(FONTS / "MPLUSRounded1c-ExtraBold.ttf"), size)
    mask, _ = font.getmask2(WORDMARK, mode="L")
    width, height = mask.size
    values = [round(mask.getpixel((x, y)) * 15 / 255) for y in range(height) for x in range(width)]
    if len(values) % 2:
        values.append(0)
    data = bytes(values[i] | values[i + 1] << 4 for i in range(0, len(values), 2))
    return (
        f"/// \"{WORDMARK}\" in M PLUS Rounded 1c ExtraBold at {size} px, cropped to its ink.\n"
        f"pub static WORDMARK_{size}: Bitmap = Bitmap {{\n    width: {width},\n    height: {height},\n"
        f"    alpha: &[\n{rust_bytes(data, '        ')}\n    ],\n}};\n"
    )


def main():
    parts = [
        "//! Generated by tools/korekore-box-fonts.py. Do not edit.\n"
        "//! The korekore box screens' type as 4-bit coverage, two pixels a byte, low nibble first.\n\n"
        "use super::text::{Bitmap, Face, Glyph};\n"
    ]
    for size, source, characters in FACES:
        parts.append(face(size, source, characters))
    for size in WORDMARK_SIZES:
        parts.append(wordmark(size))
    OUT.write_text("\n".join(parts))
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
