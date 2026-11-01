"""The generated XGGF font table must match the vendored petabyt font, and the
conversion rules (centering, deduplication, fallback cells) must hold."""
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

# The generator keeps the dashed make-cjk-font.py naming, so load it by path.
_spec = importlib.util.spec_from_file_location(
    "make_xggf_font", Path(__file__).resolve().parent / "make-xggf-font.py")
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)


class Glyphs(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.glyphs, cls.missing = gen.load_glyphs()

    def test_every_printable_ascii_character(self):
        self.assertEqual(sorted(self.glyphs), [chr(c) for c in range(0x20, 0x7F)])

    def test_unknown_upstream_characters_get_the_fallback_block(self):
        self.assertEqual(self.missing, ["$", "&", "@", "[", "\\", "]", "^", "|"])
        for key in self.missing:
            self.assertEqual(self.glyphs[key], [0x1F] * 7)

    def test_packed_shapes(self):
        # 'A' is already centered upstream, so centering leaves it alone.
        self.assertEqual(self.glyphs["A"], [0x0E, 0x11, 0x11, 0x11, 0x1F, 0x11, 0x11])
        # narrow ink is centered: the bar of 'I', the dots of ':', the stem of '!'.
        self.assertEqual(self.glyphs["I"], [0x04] * 7)
        self.assertEqual(self.glyphs[":"], [0x00, 0x04, 0x00, 0x00, 0x04, 0x00, 0x00])
        self.assertEqual(self.glyphs["!"], [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04])
        # 'g' keeps its descender inside the seven-row cell.
        self.assertEqual(self.glyphs["g"][0], 0x00)
        self.assertNotEqual(self.glyphs["g"][5], 0x00)
        self.assertNotEqual(self.glyphs["g"][6], 0x00)

    def test_first_dash_definition_wins(self):
        self.assertEqual(self.glyphs["-"], [0x00, 0x00, 0x00, 0x0E, 0x00, 0x00, 0x00])

    def test_bits_stay_in_the_low_five(self):
        for rows in self.glyphs.values():
            for row in rows:
                self.assertLess(row, 0x20)


class GeneratedFilesAreCurrent(unittest.TestCase):
    def test_generated_files_match_the_vendored_font(self):
        result = subprocess.run([sys.executable, str(gen.ROOT / "tools/make-xggf-font.py"),
                                 "--check"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
