#!/usr/bin/env python3
"""Build the fonts the korekore apps bundle, fonts/korekore/ (docs/design/korekore/README.md, "Fonts").

M PLUS Rounded 1c (weights 400, 700, 800) for what people read and DotGothic16 for the machine-ish bits, both
SIL OFL 1.1, taken from the Google Fonts repository at one commit and pinned by SHA-256, so the output is the same
on every machine. The full files are 2 to 3.6 MB each because of their Japanese glyphs, so they are cut down to:
Latin, punctuation, symbols, kana, and the Chinese characters the box's 12 px table carries
(firmware-rs/core/fonts/cjk-12px.bin: level one of the 2013 general standard table, 3,500 characters) plus any
other character the zh-Hans UI strings use.

Small pixel type (below 16 pt) is Fusion Pixel 12 px instead (Ark Pixel with Cubic 11 and Galmuri filling in, OFL;
the box's 12 px Chinese table and its small type are the same font): DotGothic16 is drawn on a 16 px grid, and at 12
to 14 pt its strokes fall between pixels and its capitals crowd. It is cut to the same characters, pinned by
SHA-256 like the others; its three licences go into one file.

Both M PLUS Rounded 1c and DotGothic16 are Japanese fonts: M PLUS Rounded 1c lacks about 1,400 of those Chinese characters (你, 这, 说…). They are
filled in from Resource Han Rounded CN (OFL; a rounded Source Han Sans), at the same weight, so Chinese text stays
in one rounded face on the Mac and on Linux alike. Both fonts use 1,000 units per em and the same ideographic box,
so the glyphs go in as they are. DotGothic16 is not filled in: a character it lacks falls back to the system font.

Subsetting and filling in are modifications under the OFL. Neither M PLUS Rounded 1c nor DotGothic16 declares a
Reserved Font Name, so their names stay; Resource Han Rounded reserves "Source", which the output does not use.
M PLUS Rounded 1c ships without a licence file in the Google Fonts repository; its OFL.txt is written from the
copyright notices in the fonts and the licence text DotGothic16 ships.

Downloads are cached in ~/.cache/vibebuddy-fonts. Needs fontTools (`pip install fonttools`) and 7-Zip's `7zz`
(`brew install sevenzip`), since Resource Han Rounded is released as a 7z archive.

Usage: tools/korekore-fonts.py
"""
import hashlib
import io
import struct
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

from fontTools import subset
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "fonts/korekore"
CACHE = Path.home() / ".cache/vibebuddy-fonts"

GOOGLE_FONTS = "https://raw.githubusercontent.com/google/fonts/e7db6ca603fd4862010383ff17ffd6cc4ffac1c0/ofl"
ROUNDED = {
    400: ("MPLUSRounded1c-Regular.ttf", "b75708b53e45b06d17d470aeeca5b766e3d1b3999f03f13ec4eb863ca846c14c"),
    700: ("MPLUSRounded1c-Bold.ttf", "c358630584e8e2d8fbd6121d0f4693255ffef6d1e6d4f3441fd6e5a963a11f9e"),
    800: ("MPLUSRounded1c-ExtraBold.ttf", "8e7c15901dca87f1451b356dda594f7d092ba252a5dcc47da74523a242493c36"),
}
DOTGOTHIC = ("DotGothic16-Regular.ttf", "3ad9af88726d42b40f7f365f0dcac785af73cf20ea6f1d5b44e57cc21150b8f1")
DOTGOTHIC_OFL = ("OFL.txt", "b6630c61ea078cacd7fabe37d14ffe557a0b45b06683374a9aa9e24262993e33")

FUSION = ("https://github.com/TakWolf/fusion-pixel-font/releases/download/2026.09.25/"
          "fusion-pixel-font-12px-proportional-ttf-v2026.09.25.zip",
          "47585b9c4b6d8d57380e9dec75bc7e8043fdbfa77d1843c3aaa19a1e7bd15d50")
FUSION_MEMBER = "fusion-pixel-12px-proportional-latin.ttf"
FUSION_OUTPUT = "FusionPixel12-Regular.ttf"
FUSION_LICENSES = ["LICENSES/ark-pixel/OFL.txt", "LICENSES/cubic-11/OFL.txt", "LICENSES/galmuri/LICENSE.txt"]
RESOURCE_HAN = ("https://github.com/CyanoHao/Resource-Han-Rounded/releases/download/v1.910/RHR-CFF2-CN-1.910.7z",
                "4ad7b141535a1f11831287b0a6f71ddcec8daa92dc1d82c59892068f8ae5df09")
RESOURCE_HAN_MEMBER = "ResourceHanRoundedCN-VF.otf"
RESOURCE_HAN_NOTICE = ("Copyright © 2018—2022 Cyano Hao.\n"
                       "Portions © 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'.")
# Resource Han Rounded's axes: weight 200 to 900, and how rounded (100 is fully, its default).
RESOURCE_HAN_ROUND = 100

CJK_TABLE = ROOT / "firmware-rs/core/fonts/cjk-12px.bin"
# Every character in these is kept, whatever block it is in.
UI_STRINGS = [ROOT / "app/Localization/zh-Hans.lproj/Localizable.strings", ROOT / "desktop/src/i18n.rs"]
RANGES = [
    (0x0020, 0x007E),  # ASCII
    (0x00A0, 0x017F),  # Latin-1 and Latin Extended-A
    (0x2000, 0x206F),  # general punctuation: dashes, quotes, the ellipsis, the bullet
    (0x2190, 0x21FF),  # arrows
    (0x2500, 0x25FF),  # box drawing, blocks and geometric shapes: ▸ ▷ ●
    (0x2600, 0x26FF),  # miscellaneous symbols: ★ ☆
    (0x3000, 0x30FF),  # CJK punctuation, hiragana, katakana
    (0xFF00, 0xFFEF),  # full-width forms
]


# Fonts keep their sources' timestamps (recalcTimestamp=False), so the output is the same byte for byte every run.


def fetch(url, sha256):
    cached = CACHE / url.rsplit("/", 1)[1]
    if not cached.exists():
        CACHE.mkdir(parents=True, exist_ok=True)
        print(f"downloading {url}", file=sys.stderr)
        with urllib.request.urlopen(url) as response:
            cached.write_bytes(response.read())
    data = cached.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest != sha256:
        sys.exit(f"{cached}: SHA-256 {digest}, expected {sha256}")
    return cached


def google_font(directory, name, sha256):
    return fetch(f"{GOOGLE_FONTS}/{directory}/{name}", sha256)


def resource_han():
    archive = fetch(*RESOURCE_HAN)
    data = subprocess.run(["7zz", "e", "-so", str(archive), RESOURCE_HAN_MEMBER], check=True,
                          capture_output=True).stdout
    return TTFont(io.BytesIO(data))


def characters():
    codepoints = set()
    for first, last in RANGES:
        codepoints.update(range(first, last + 1))
    table = CJK_TABLE.read_bytes()
    (count,) = struct.unpack_from("<H", table)
    codepoints.update(struct.unpack_from(f"<{count}H", table, 2))
    for path in UI_STRINGS:
        codepoints.update(ord(c) for c in path.read_text() if ord(c) > 0x7F)
    return sorted(codepoints)


def cut(font, codepoints):
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    options.hinting = False
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=codepoints)
    subsetter.subset(font)
    return font


def fill_in(font, donor, location, codepoints):
    """Copy the variable donor's glyphs for `codepoints` at `location` into `font`, outlines converted to TrueType's
    quadratic curves. Its ideographs keep one advance at every weight, so the default metrics hold."""
    glyphs, donor_cmap = donor.getGlyphSet(location=location), donor.getBestCmap()
    order = list(font.getGlyphOrder())
    unicode_tables = [table for table in font["cmap"].tables if table.isUnicode() and table.format in (4, 12)]
    for codepoint in codepoints:
        source, name = donor_cmap[codepoint], f"uni{codepoint:04X}"
        pen = TTGlyphPen(None)
        glyphs[source].draw(Cu2QuPen(pen, max_err=1.0, reverse_direction=True))
        glyph = pen.glyph()
        glyph.recalcBounds(font["glyf"])
        font["glyf"][name] = glyph
        font["hmtx"][name] = (donor["hmtx"][source][0], glyph.xMin if glyph.numberOfContours else 0)
        font["vmtx"][name] = donor["vmtx"][source]
        order.append(name)
        for table in unicode_tables:
            table.cmap[codepoint] = name
    font.setGlyphOrder(order)
    font["glyf"].glyphOrder = order
    font["OS/2"].recalcUnicodeRanges(font)
    for record in font["name"].names:
        if record.nameID == 0:
            record.string = f"{record.toUnicode()} {RESOURCE_HAN_NOTICE}".replace("\n", " ")


def one_family(font, style):
    """Google's Bold file names its family "Rounded Mplus 1c Bold" and gives no typographic family, so a lookup by
    family and weight misses it. Give every weight the typographic family the other two already share."""
    for name_id, value in [(16, "Rounded Mplus 1c"), (17, style)]:
        font["name"].setName(value, name_id, 3, 1, 0x409)


def licence(copyright_notice, ofl):
    """The OFL text DotGothic16 ships, under other fonts' copyright lines."""
    body = ofl[ofl.index("This Font Software is licensed"):]
    return f"{copyright_notice}\n\n{body}"


def main():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    codepoints = characters()
    rounded = {weight: cut(TTFont(google_font("mplusrounded1c", *source), recalcTimestamp=False), codepoints)
               for weight, source in ROUNDED.items()}
    rounded_copyright = rounded[400]["name"].getDebugName(0)
    donor = resource_han()
    missing = sorted(set(codepoints) - set(rounded[400].getBestCmap()) & set(donor.getBestCmap()))
    print(f"filling in {len(missing)} characters from Resource Han Rounded", file=sys.stderr)
    donor = cut(donor, missing)
    for weight, font in rounded.items():
        fill_in(font, donor, {"wght": weight, "ROND": RESOURCE_HAN_ROUND}, missing)
        one_family(font, ROUNDED[weight][0].removeprefix("MPLUSRounded1c-").removesuffix(".ttf"))
        font.save(OUTPUT / ROUNDED[weight][0])
    cut(TTFont(google_font("dotgothic16", *DOTGOTHIC), recalcTimestamp=False), codepoints).save(OUTPUT / DOTGOTHIC[0])
    fusion = zipfile.ZipFile(fetch(*FUSION))
    cut(TTFont(io.BytesIO(fusion.read(FUSION_MEMBER)), recalcTimestamp=False), codepoints).save(OUTPUT / FUSION_OUTPUT)
    (OUTPUT / "OFL-FusionPixel.txt").write_text(
        "\n\n".join(f"{name}\n\n{fusion.read(name).decode()}" for name in FUSION_LICENSES))
    for name in [*(source for source, _ in ROUNDED.values()), DOTGOTHIC[0], FUSION_OUTPUT]:
        print(f"{name}: {(OUTPUT / name).stat().st_size} bytes", file=sys.stderr)
    ofl = google_font("dotgothic16", *DOTGOTHIC_OFL).read_text()
    (OUTPUT / "OFL-DotGothic16.txt").write_text(ofl)
    (OUTPUT / "OFL-MPLUSRounded1c.txt").write_text(licence(f"{rounded_copyright}\n{RESOURCE_HAN_NOTICE}", ofl))


if __name__ == "__main__":
    main()
