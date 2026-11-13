#!/usr/bin/env python3
"""Build the edition image of a company edition (ADR-0025), which the factory writes to the box's `edition` partition.

Usage:
    tools/make_edition.py <character dir> <output.bin>

The Character directory has the layout of characters/<id>/ (in the private assets repo, never this one) with
pack.bin already built by tools/make-character.sh, and an edition.json:

    {"id": "acme", "name": "Acme Bot", "summary": "One sentence that says who it is.", "logo": "logo.png"}

The id must be the pack's Character id. "logo" is optional: an image in the same directory that the box shows for
about 2 s after it is switched on. It is scaled to fit the 320 x 240 screen; any border left over takes the color
of the image's top-left pixel, and transparent pixels become black. Needs Pillow. The image is checked before it is written. Byte layout: see
firmware-rs/core/src/edition.rs.
"""

from __future__ import annotations

import json
import struct
import sys
import zlib
from pathlib import Path

MAGIC = b"VBED"
VERSION = 1
HEADER_BYTES = 512
PACK_OFFSET = 4096
PARTITION_BYTES = 0x200000
ID_AT, NAME_AT, SUMMARY_AT, LOGO_AT = 8, 40, 104, 360
CRC_AT = HEADER_BYTES - 4
PACK_HEADER_BYTES = 1024
SCREEN_WIDTH, SCREEN_HEIGHT = 320, 240
LOGO_HEADER_BYTES = 8
MAX_RUN = 0xFFFF


def encode_logo(path: Path) -> bytes:
    """A boot logo from an image file: fitted to the screen, in RGB565 runs (see edition.rs)."""
    from PIL import Image

    with Image.open(path) as source:
        image = source.convert("RGBA")
    flat = Image.new("RGBA", image.size, (0, 0, 0, 255))
    flat.alpha_composite(image)
    flat = flat.convert("RGB")
    scale = min(SCREEN_WIDTH / flat.width, SCREEN_HEIGHT / flat.height)
    size = (max(1, round(flat.width * scale)), max(1, round(flat.height * scale)))
    screen = Image.new("RGB", (SCREEN_WIDTH, SCREEN_HEIGHT), flat.getpixel((0, 0)))
    screen.paste(flat.resize(size, Image.Resampling.LANCZOS), ((SCREEN_WIDTH - size[0]) // 2, (SCREEN_HEIGHT - size[1]) // 2))
    rgb = screen.tobytes()
    return encode_runs([(rgb[at] >> 3) << 11 | (rgb[at + 1] >> 2) << 5 | rgb[at + 2] >> 3 for at in range(0, len(rgb), 3)])


def encode_runs(pixels: list[int]) -> bytes:
    """RGB565 pixels of the whole screen, row by row, as a logo."""
    if len(pixels) != SCREEN_WIDTH * SCREEN_HEIGHT:
        raise ValueError(f"a logo has {SCREEN_WIDTH * SCREEN_HEIGHT} pixels, not {len(pixels)}")
    logo = bytearray(b"LOGO" + struct.pack("<HH", SCREEN_WIDTH, SCREEN_HEIGHT))
    start = 0
    while start < len(pixels):
        end = start
        while end < len(pixels) and end - start < MAX_RUN and pixels[end] == pixels[start]:
            end += 1
        logo += struct.pack("<HH", end - start, pixels[start])
        start = end
    return bytes(logo)


def check_logo(logo: bytes) -> None:
    if logo[:4] != b"LOGO" or struct.unpack_from("<HH", logo, 4) != (SCREEN_WIDTH, SCREEN_HEIGHT):
        raise ValueError(f"the logo is not {SCREEN_WIDTH} x {SCREEN_HEIGHT}")
    runs = logo[LOGO_HEADER_BYTES:]
    if len(runs) % 4:
        raise ValueError("the logo's runs are cut short")
    counts = [struct.unpack_from("<H", runs, at)[0] for at in range(0, len(runs), 4)]
    if 0 in counts or sum(counts) != SCREEN_WIDTH * SCREEN_HEIGHT:
        raise ValueError("the logo's runs do not fill the screen")


def _field(text: str, size: int, what: str) -> bytes:
    encoded = text.encode("utf-8")
    if len(encoded) >= size:
        raise ValueError(f"{what} is longer than {size - 1} bytes in UTF-8")
    return encoded


def pack_id(pack: bytes) -> str:
    """The Character id of a checked Character pack; ValueError when it isn't one."""
    if len(pack) < PACK_HEADER_BYTES or pack[:4] != b"VBCP":
        raise ValueError("not a Character pack")
    if struct.unpack_from("<I", pack, 1020)[0] != zlib.crc32(pack[:1020]):
        raise ValueError("the Character pack's header CRC is wrong")
    length, crc = struct.unpack_from("<II", pack, 8)
    if len(pack) != PACK_HEADER_BYTES + length or zlib.crc32(pack[PACK_HEADER_BYTES:]) != crc:
        raise ValueError("the Character pack's payload is damaged")
    return pack[16:48].split(b"\0", 1)[0].decode("ascii")


def build(edition_id: str, name: str, summary: str, pack: bytes, logo: bytes | None = None) -> bytes:
    if not edition_id or not edition_id.isascii() or not all(33 <= ord(char) <= 126 for char in edition_id):
        raise ValueError("the edition id must be printable ASCII")
    if not name:
        raise ValueError("the name is empty")
    if pack_id(pack) != edition_id:
        raise ValueError(f"the pack is Character {pack_id(pack)!r}, not {edition_id!r}")
    header = bytearray(PACK_OFFSET)
    header[:4] = MAGIC
    struct.pack_into("<I", header, 4, VERSION)
    for at, text, size, what in (
        (ID_AT, edition_id, NAME_AT - ID_AT, "the id"),
        (NAME_AT, name, SUMMARY_AT - NAME_AT, "the name"),
        (SUMMARY_AT, summary, LOGO_AT - SUMMARY_AT, "the summary"),
    ):
        encoded = _field(text, size, what)
        header[at : at + len(encoded)] = encoded
    body = pack
    if logo:
        check_logo(logo)
        # The firmware reads flash 4 bytes at a time, so the logo starts on a multiple of 4.
        body += bytes(-len(body) % 4)
        struct.pack_into("<II", header, LOGO_AT, PACK_OFFSET + len(body), len(logo))
        body += logo
    struct.pack_into("<I", header, CRC_AT, zlib.crc32(bytes(header[:CRC_AT])))
    image = bytes(header) + body
    if len(image) > PARTITION_BYTES:
        raise ValueError(f"the image is {len(image)} bytes; the edition partition holds {PARTITION_BYTES}")
    return image


def pack_of(image: bytes) -> bytes:
    """The Character pack inside an edition image, without the logo after it."""
    if len(image) < PACK_OFFSET + PACK_HEADER_BYTES:
        raise ValueError("the edition image has no Character pack")
    payload_length = struct.unpack_from("<I", image, PACK_OFFSET + 8)[0]
    return image[PACK_OFFSET : PACK_OFFSET + PACK_HEADER_BYTES + payload_length]


def check(image: bytes) -> str:
    """Checks an edition image as the firmware does; returns its id."""
    if image[:4] != MAGIC or struct.unpack_from("<I", image, 4)[0] != VERSION:
        raise ValueError("not an edition image")
    if struct.unpack_from("<I", image, CRC_AT)[0] != zlib.crc32(image[:CRC_AT]):
        raise ValueError("the edition header CRC is wrong")
    edition_id = image[ID_AT:NAME_AT].split(b"\0", 1)[0].decode("ascii")
    if pack_id(pack_of(image)) != edition_id:
        raise ValueError("the pack is not the edition's Character")
    logo_at, logo_length = struct.unpack_from("<II", image, LOGO_AT)
    if logo_length:
        if logo_at % 4 or logo_at + logo_length > len(image):
            raise ValueError("the logo is not where the header says")
        check_logo(image[logo_at : logo_at + logo_length])
    return edition_id


def main(argv: list[str]) -> None:
    if len(argv) != 3:
        sys.exit(__doc__)
    directory, output = Path(argv[1]), Path(argv[2])
    manifest = json.loads((directory / "edition.json").read_text(encoding="utf-8"))
    logo = encode_logo(directory / manifest["logo"]) if manifest.get("logo") else None
    image = build(manifest["id"], manifest["name"], manifest.get("summary", ""), (directory / "pack.bin").read_bytes(), logo)
    check(image)
    output.write_bytes(image)
    print(f"{output}: {len(image)} bytes, edition {manifest['id']}" + (f", logo {len(logo)} bytes" if logo else ""))


if __name__ == "__main__":
    main(sys.argv)
