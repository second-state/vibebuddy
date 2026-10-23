"""The korekore code generated from docs/design/korekore must match it: edit the design, then run
tools/korekore-codegen.py and commit what it writes."""
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class GeneratedCodeIsCurrent(unittest.TestCase):
    def test_generated_files_match_the_design(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/korekore-codegen.py"), "--check"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
