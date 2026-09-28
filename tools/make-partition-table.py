#!/usr/bin/env python3
"""Compile firmware/partitions.csv into an ESP-IDF partition table binary (flashed at 0x8000).

espflash's partition table parser rejects custom data subtypes (voices is 0x40) with a panic, so the
Rust firmware build compiles the table itself. The output is byte-identical to ESP-IDF's
gen_esp32part.py: 32-byte entries, then an MD5 entry, padded with 0xFF to 0xC00.

Usage: make-partition-table.py <partitions.csv> <output.bin>
"""

import hashlib
import struct
import sys

TYPES = {"app": 0x00, "data": 0x01}
SUBTYPES = {
    "app": {"factory": 0x00, "test": 0x20, **{f"ota_{n}": 0x10 + n for n in range(16)}},
    "data": {"ota": 0x00, "phy": 0x01, "nvs": 0x02, "coredump": 0x03, "nvs_keys": 0x04, "efuse": 0x05, "undefined": 0x06, "esphttpd": 0x80, "fat": 0x81, "spiffs": 0x82, "littlefs": 0x83},
}
TABLE_OFFSET = 0x8000
TABLE_BYTES = 0xC00
FIRST_OFFSET = TABLE_OFFSET + 0x1000
APP_ALIGN = 0x10000
DATA_ALIGN = 0x1000


def number(text):
    text = text.strip()
    for suffix, scale in (("K", 1024), ("M", 1024 * 1024)):
        if text.upper().endswith(suffix):
            return int(text[:-1], 0) * scale
    return int(text, 0)


def parse(path):
    rows = []
    offset = FIRST_OFFSET
    with open(path, encoding="utf-8") as source:
        for line in source:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            fields = [field.strip() for field in line.split(",")]
            fields += [""] * (6 - len(fields))
            name, kind, subtype, at, size, flags = fields[:6]
            kind_value = TYPES[kind] if kind in TYPES else int(kind, 0)
            table = SUBTYPES.get(kind, {})
            subtype_value = table[subtype] if subtype in table else int(subtype, 0)
            align = APP_ALIGN if kind_value == TYPES["app"] else DATA_ALIGN
            if at:
                offset = number(at)
            else:
                offset = (offset + align - 1) // align * align
            length = number(size)
            flag_value = 1 if "encrypted" in flags else 0
            rows.append((name, kind_value, subtype_value, offset, length, flag_value))
            offset += length
    return rows


def encode(rows):
    table = b""
    for name, kind, subtype, offset, size, flags in rows:
        label = name.encode("ascii")
        assert len(label) <= 16, f"partition name too long: {name}"
        table += struct.pack("<2sBBLL16sL", b"\xaa\x50", kind, subtype, offset, size, label, flags)
    table += b"\xeb\xeb" + b"\xff" * 14 + hashlib.md5(table).digest()
    assert len(table) <= TABLE_BYTES
    return table + b"\xff" * (TABLE_BYTES - len(table))


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    rows = parse(sys.argv[1])
    with open(sys.argv[2], "wb") as out:
        out.write(encode(rows))
    for name, _, _, offset, size, _ in rows:
        print(f"{name:<10} 0x{offset:06x} {size // 1024:>6} KB")


if __name__ == "__main__":
    main()
