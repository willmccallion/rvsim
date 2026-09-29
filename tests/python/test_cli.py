"""Tests for the ``python -m rvsim`` command line.

Run with: .venv/bin/python -m unittest discover -s tests/python
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


class Bench(unittest.TestCase):
    def bench(self, *args):
        return subprocess.run(
            [sys.executable, "-m", "rvsim", "bench", *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )

    def test_list_names_each_benchmark_and_its_command(self):
        result = self.bench("--list")

        self.assertEqual(result.returncode, 0)
        self.assertIn("coremark", result.stdout)
        self.assertIn("dhrystone 200000", result.stdout)

    def test_an_unknown_benchmark_is_an_error(self):
        result = self.bench("not-a-benchmark")

        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown benchmarks", result.stderr)


if __name__ == "__main__":
    unittest.main()
