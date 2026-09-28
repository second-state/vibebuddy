#!/usr/bin/env python3
"""Tests for the voice pack packer: the byte layout must match what the firmware's agent_voice_pack.h describes."""

from __future__ import annotations

import struct
import unittest
import zlib

import make_voice_pack


class VoicePackLayout(unittest.TestCase):
    def setUp(self) -> None:
        self.clips = [b"\x01" * 10, b"\x02" * 20, b"\x03" * 30, b"\x04" * 40, b"\x05" * 50]
        self.pack = make_voice_pack.build("wanwanxiaohe", self.clips)

    def test_header_is_256_bytes_followed_by_the_five_clips_in_order(self) -> None:
        self.assertEqual(self.pack[256:], b"".join(self.clips))
        self.assertEqual(len(self.pack), 256 + 150)

    def test_magic_version_lengths_and_id(self) -> None:
        self.assertEqual(self.pack[0:4], b"VBVP")
        self.assertEqual(struct.unpack_from("<I", self.pack, 4)[0], 1)
        self.assertEqual(struct.unpack_from("<I", self.pack, 8)[0], 150)
        self.assertEqual(self.pack[16:48], b"wanwanxiaohe".ljust(32, b"\0"))

    def test_clip_table_points_into_the_payload(self) -> None:
        offsets = struct.unpack_from("<5I", self.pack, 48)
        lengths = struct.unpack_from("<5I", self.pack, 68)
        self.assertEqual(offsets, (256, 266, 286, 316, 356))
        self.assertEqual(lengths, (10, 20, 30, 40, 50))

    def test_crcs_are_zlib_crc32(self) -> None:
        self.assertEqual(struct.unpack_from("<I", self.pack, 12)[0], zlib.crc32(self.pack[256:]))
        self.assertEqual(struct.unpack_from("<I", self.pack, 88)[0], zlib.crc32(self.pack[:88]))
        self.assertEqual(self.pack[92:256], bytes(164))

    def test_voice_id_must_fit_and_clips_must_not_be_empty(self) -> None:
        with self.assertRaises(ValueError):
            make_voice_pack.build("x" * 32, self.clips)
        with self.assertRaises(ValueError):
            make_voice_pack.build("ok", self.clips[:4] + [b""])


if __name__ == "__main__":
    unittest.main()
