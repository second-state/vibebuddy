#!/usr/bin/env python3
"""Build the edition image of a company edition (ADR-0025), which the factory writes to the box's `edition` partition.

Usage:
    tools/make_edition.py <character dir> <output.bin>

The Character directory has the layout of characters/<id>/ (in the private assets repo, never this one) with
pack.bin already built by tools/make-character.sh, and an edition.json:

    {"id": "acme", "name": "Acme Bot", "summary": "One sentence that says who it is."}

The id must be the pack's Character id. The image is checked before it is written. Byte layout: see
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


def build(edition_id: str, name: str, summary: str, pack: bytes) -> bytes:
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
    struct.pack_into("<I", header, CRC_AT, zlib.crc32(bytes(header[:CRC_AT])))
    image = bytes(header) + pack
    if len(image) > PARTITION_BYTES:
        raise ValueError(f"the image is {len(image)} bytes; the edition partition holds {PARTITION_BYTES}")
    return image


def check(image: bytes) -> str:
    """Checks an edition image as the firmware does; returns its id."""
    if image[:4] != MAGIC or struct.unpack_from("<I", image, 4)[0] != VERSION:
        raise ValueError("not an edition image")
    if struct.unpack_from("<I", image, CRC_AT)[0] != zlib.crc32(image[:CRC_AT]):
        raise ValueError("the edition header CRC is wrong")
    edition_id = image[ID_AT:NAME_AT].split(b"\0", 1)[0].decode("ascii")
    if pack_id(image[PACK_OFFSET:]) != edition_id:
        raise ValueError("the pack is not the edition's Character")
    return edition_id


def main(argv: list[str]) -> None:
    if len(argv) != 3:
        sys.exit(__doc__)
    directory, output = Path(argv[1]), Path(argv[2])
    manifest = json.loads((directory / "edition.json").read_text(encoding="utf-8"))
    image = build(manifest["id"], manifest["name"], manifest.get("summary", ""), (directory / "pack.bin").read_bytes())
    check(image)
    output.write_bytes(image)
    print(f"{output}: {len(image)} bytes, edition {manifest['id']}")


if __name__ == "__main__":
    main(sys.argv)
