"""Tests for the ``python -m rvsim`` command line.

Run with: .venv/bin/python -m unittest discover -s testing/python
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class JsonExport(unittest.TestCase):
    def test_json_flag_writes_the_run_stats(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "stats.json")

            subprocess.run(
                [sys.executable, "-m", "rvsim", FIB, "--json", out, "--quiet"],
                cwd=ROOT,
                check=True,
                capture_output=True,
            )

            with open(out) as f:
                stats = json.load(f)
        self.assertGreater(stats["cycles"], 0)
        self.assertGreater(stats["instructions_retired"], 0)
        self.assertIn("core0.cache.l1d.hits", stats)


if __name__ == "__main__":
    unittest.main()
