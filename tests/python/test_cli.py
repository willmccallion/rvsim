"""Tests for the ``python -m rvsim`` command line.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

from rvsim.presets import PRESETS

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


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class ProgramOutput(unittest.TestCase):
    def test_every_preset_prints_all_the_output_before_the_exit(self):
        for preset in PRESETS:
            with self.subTest(preset=preset):
                result = subprocess.run(
                    [
                        sys.executable,
                        "-m",
                        "rvsim",
                        FIB,
                        "--preset",
                        preset,
                        "--no-stats",
                    ],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    check=True,
                )

                self.assertEqual(result.stdout, "fib(20)=6765\n")


class Preset(unittest.TestCase):
    def test_an_unknown_preset_lists_every_registered_preset(self):
        result = subprocess.run(
            [sys.executable, "-m", "rvsim", "x.elf", "--preset", "not-a-preset"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 2)
        for name in PRESETS:
            self.assertIn(name, result.stderr)


class Bench(unittest.TestCase):
    def bench(self, *args):
        return subprocess.run(
            [sys.executable, "-m", "rvsim", "bench", *args],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
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

    def test_an_unknown_preset_lists_every_registered_preset(self):
        result = self.bench("--preset", "not-a-preset")

        self.assertEqual(result.returncode, 2)
        for name in PRESETS:
            self.assertIn(name, result.stderr)


if __name__ == "__main__":
    unittest.main()
