#!/usr/bin/env python3
"""Draw the korekore box screens' type into firmware tables (ADR-0021, docs/design/korekore).

The design sets the box screens in DotGothic16 at 9 to 38 px. On the box, thin pixel fonts read as faint next to
the robot screens' blocky type, so the korekore screens draw every size in the robot's own 5 × 7 font
(firmware-rs/core/src/canvas.rs), whole pixels only, at a whole multiple of it:
  - 9 to 18 px: 5 × 7
  - 22 px: 5 × 7, doubled
  - 30 to 36 px: 5 × 7, tripled
  - 38 px: 5 × 7, four times
Like the robot screens, the font has capitals only: a lowercase letter draws as its capital. This tool reads the
letter and digit tables from canvas.rs and adds the punctuation the robot draws in glyph_row, plus a few more marks
the korekore screens use. Every character is 5 columns wide with one blank column after it. The "korekore"
wordmark is M PLUS Rounded 1c ExtraBold, kept smooth: a rounded logo, not text.

The output is firmware-rs/core/src/korekore/glyphs.rs: each face's glyphs as 4-bit coverage (0 = none, 15 = full),
which the text faces only ever use as 0 or 15. The wordmark goes through FreeType in Pillow, so a Pillow upgrade
could move a pixel: regenerate and commit, don't compare in CI.

Usage: tools/korekore-box-fonts.py
"""
import re
from pathlib import Path

from PIL import ImageFont

ROOT = Path(__file__).resolve().parents[1]
FONTS = ROOT / "fonts/korekore"
CANVAS = ROOT / "firmware-rs/core/src/canvas.rs"
OUT = ROOT / "firmware-rs/core/src/korekore/glyphs.rs"

ASCII = "".join(chr(code) for code in range(0x20, 0x7F)) + "·›‹…×"
# The design's size, how many times the 5 × 7 font is scaled to draw it, and the characters it needs.
FACES = [
    (9, 1, ASCII),
    (10, 1, ASCII),
    (11, 1, ASCII),
    (12, 1, ASCII),
    (13, 1, ASCII),
    (14, 1, ASCII),
    (16, 1, ASCII),
    (18, 1, ASCII),
    (22, 2, ASCII),
    (30, 3, " 0123456789:ABCEFKORSU"),
    (34, 3, "!"),
    (36, 3, "YAT!"),
    (38, 4, "0123456789:"),
]
WORDMARK = "korekore"
WORDMARK_SIZES = [12, 14, 40]
# Blank rows above the capitals and below the baseline, in 5 × 7 pixels.
ABOVE, BELOW = 2, 2

# The marks the robot draws in canvas.rs glyph_row, and the ones only korekore needs. Seven rows each, bit 4 the
# left column.
MARKS = {
    "-": [0, 0, 0, 0x1F, 0, 0, 0],
    ".": [0, 0, 0, 0, 0, 0, 0x04],
    ",": [0, 0, 0, 0, 0, 0x04, 0x08],
    "'": [0x04, 0x04, 0, 0, 0, 0, 0],
    ":": [0, 0, 0x04, 0, 0, 0x04, 0],
    "/": [0x10 >> min(row, 4) if row < 5 else 0x01 for row in range(7)],
    "!": [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
    "?": [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
    ">": [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10],
    "_": [0, 0, 0, 0, 0, 0, 0x1F],
    "%": [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
    "<": [0x01, 0x02, 0x04, 0x08, 0x04, 0x02, 0x01],
    "\"": [0x0A, 0x0A, 0, 0, 0, 0, 0],
    "#": [0x0A, 0x1F, 0x0A, 0x0A, 0x0A, 0x1F, 0x0A],
    "$": [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
    "&": [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
    "(": [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
    ")": [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
    "*": [0, 0x04, 0x15, 0x0E, 0x15, 0x04, 0],
    "+": [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
    ";": [0, 0, 0x04, 0, 0, 0x04, 0x08],
    "=": [0, 0, 0x1F, 0, 0x1F, 0, 0],
    "@": [0x0E, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0E],
    "[": [0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E],
    "\\": [0x10, 0x10, 0x08, 0x04, 0x02, 0x01, 0x01],
    "]": [0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E],
    "^": [0x04, 0x0A, 0x11, 0, 0, 0, 0],
    "`": [0x08, 0x04, 0, 0, 0, 0, 0],
    "{": [0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02],
    "|": [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
    "}": [0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08],
    "~": [0, 0, 0x08, 0x15, 0x02, 0, 0],
    "·": [0, 0, 0, 0x04, 0, 0, 0],
    "›": [0, 0x08, 0x04, 0x02, 0x04, 0x08, 0],
    "‹": [0, 0x02, 0x04, 0x08, 0x04, 0x02, 0],
    "…": [0, 0, 0, 0, 0, 0, 0x15],
    "×": [0, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0],
}


def robot_font():
    """The 5 × 7 rows of every character korekore can draw: {character: seven rows of five bits}."""
    source = CANVAS.read_text()

    def table(name):
        body = re.search(rf"const {name}: \[\[u8; 7\]; \d+\] = \[(.*?)\n\];", source, re.S).group(1)
        return [[int(value, 16) for value in re.findall(r"0x[0-9a-fA-F]+", row)] for row in re.findall(r"\[([^\]]*)\]", body)]

    letters, digits = table("LETTER_GLYPHS"), table("DIGIT_GLYPHS")
    font = {" ": [0] * 7, **MARKS}
    for index, rows in enumerate(letters):
        font[chr(ord("A") + index)] = font[chr(ord("a") + index)] = rows
    for index, rows in enumerate(digits):
        font[chr(ord("0") + index)] = rows
    return font


def five_by_seven(characters, scale):
    """The 5 × 7 font scaled by a whole number: {character: (advance, left, top, rows of 0/1)}, ascent, descent."""
    font = robot_font()
    glyphs = {}
    for character in characters:
        if character not in font:
            continue
        rows = [[(bits >> (4 - x)) & 1 for x in range(5)] for bits in font[character]]
        rows = [[pixel for pixel in row for _ in range(scale)] for row in rows for _ in range(scale)]
        glyphs[character] = (6 * scale, 0, ABOVE * scale, rows)
    return glyphs, (ABOVE + 7) * scale, BELOW * scale


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


def face(size, scale, characters):
    glyphs, ascent, descent = five_by_seven(characters, scale)
    what = "the 5 × 7 font" + (f", ×{scale}" if scale > 1 else "")
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
    for size, scale, characters in FACES:
        parts.append(face(size, scale, characters))
    for size in WORDMARK_SIZES:
        parts.append(wordmark(size))
    OUT.write_text("\n".join(parts))
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
