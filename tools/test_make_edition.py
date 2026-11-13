#!/usr/bin/env python3
"""Tests for the edition image builder: its output must be what firmware-rs/core/src/edition.rs reads."""

from __future__ import annotations

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path

import make_edition

SAMPLE_PACK = Path(__file__).resolve().parent.parent / "firmware-rs/core/src/fixtures/sample_character_pack.bin"
SAMPLE_EDITION = SAMPLE_PACK.with_name("sample_edition.bin")
TWO_HALVES = [0xF800] * (320 * 120) + [0x001F] * (320 * 120)


class EditionImage(unittest.TestCase):
    def setUp(self) -> None:
        self.pack = SAMPLE_PACK.read_bytes()
        self.image = make_edition.build("sample", "Sample Co", "Made for tests.", self.pack)

    def test_the_pack_follows_the_header_at_4096(self) -> None:
        self.assertEqual(self.image[:4], b"VBED")
        self.assertEqual(self.image[4096:], self.pack)
        self.assertEqual(make_edition.check(self.image), "sample")

    def test_the_fields_are_nul_terminated_where_the_firmware_reads_them(self) -> None:
        self.assertEqual(self.image[8:40], b"sample".ljust(32, b"\0"))
        self.assertEqual(self.image[40:104], b"Sample Co".ljust(64, b"\0"))
        self.assertEqual(self.image[104:360], b"Made for tests.".ljust(256, b"\0"))
        self.assertEqual(struct.unpack_from("<II", self.image, 360), (0, 0), "no logo")

    def test_the_firmware_fixture_is_this_builders_output(self) -> None:
        self.assertEqual(SAMPLE_EDITION.read_bytes(), self.image)

    def test_a_pack_of_another_character_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "not 'acme'"):
            make_edition.build("acme", "Acme", "", self.pack)

    def test_a_damaged_pack_is_refused(self) -> None:
        damaged = self.pack[:-1] + bytes([self.pack[-1] ^ 1])
        with self.assertRaisesRegex(ValueError, "damaged"):
            make_edition.build("sample", "Sample Co", "", damaged)

    def test_a_bad_manifest_is_refused(self) -> None:
        for edition_id, name, summary in (("", "x", ""), ("a b", "x", ""), ("sample", "", ""), ("sample", "x" * 64, ""), ("sample", "x", "y" * 256)):
            with self.subTest(edition_id=edition_id, name=name[:5], summary=summary[:5]), self.assertRaises(ValueError):
                make_edition.build(edition_id, name, summary, self.pack)

    def test_a_logo_follows_the_pack_on_a_multiple_of_4(self) -> None:
        logo = make_edition.encode_runs(TWO_HALVES)
        image = make_edition.build("sample", "Sample Co", "", self.pack, logo)
        logo_at, logo_length = struct.unpack_from("<II", image, 360)
        self.assertEqual(logo_at % 4, 0)
        self.assertEqual(image[logo_at:], logo)
        self.assertEqual(logo_length, len(logo))
        self.assertEqual(make_edition.pack_of(image), self.pack, "the factory writes only the pack to voices")
        self.assertEqual(make_edition.check(image), "sample")

    def test_the_runs_are_those_the_firmware_tests_draw(self) -> None:
        # The logo() of firmware-rs/core/tests/firmware.rs: red on the top half, blue below.
        expected = b"LOGO" + struct.pack("<HH", 320, 240) + struct.pack("<HHHH", 38400, 0xF800, 38400, 0x001F)
        self.assertEqual(make_edition.encode_runs(TWO_HALVES), expected)

    def test_a_long_run_is_split(self) -> None:
        runs = make_edition.encode_runs([0x1234] * (320 * 240))[8:]
        counts = [struct.unpack_from("<H", runs, at)[0] for at in range(0, len(runs), 4)]
        self.assertEqual(counts, [65535, 320 * 240 - 65535])

    @unittest.skipUnless(importlib.util.find_spec("PIL"), "needs Pillow, as a logo does")
    def test_an_image_is_fitted_to_the_screen(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as directory:
            # Half the screen's height: it is scaled to the full width and centered, the border in its corner color.
            path = Path(directory) / "logo.png"
            Image.new("RGB", (160, 60), (255, 255, 255)).save(path)
            runs = make_edition.encode_logo(path)[8:]
        counts = [struct.unpack_from("<HH", runs, at) for at in range(0, len(runs), 4)]
        self.assertEqual(counts, [(65535, 0xFFFF), (320 * 240 - 65535, 0xFFFF)])

    def test_a_logo_that_does_not_fill_the_screen_is_refused(self) -> None:
        short = make_edition.encode_runs(TWO_HALVES)[:-4]
        with self.assertRaisesRegex(ValueError, "fill the screen"):
            make_edition.build("sample", "Sample Co", "", self.pack, short)

    def test_a_damaged_image_fails_the_check(self) -> None:
        damaged = bytearray(self.image)
        damaged[50] ^= 1
        with self.assertRaisesRegex(ValueError, "CRC"):
            make_edition.check(bytes(damaged))


if __name__ == "__main__":
    unittest.main()
