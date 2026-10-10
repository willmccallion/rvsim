"""A bare-metal program that prints and exits through HTIF behaves the
same on rvsim as on spike, whose proxy serves the same protocol.

Needs the HTIF variant of the bundled programs (``make software``) and
the local spike (``make riscv-tests-build`` builds it); skipped otherwise.
"""

import os
import subprocess
import sys
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SPIKE = os.path.join(ROOT, "tests", "builds", "spike-install", "bin", "spike")
PROGRAMS = os.path.join(ROOT, "software", "bin-htif", "programs")
MANDELBROT = os.path.join(PROGRAMS, "mandelbrot.elf")


def run_spike(elf: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [SPIKE, "--isa=rv64gcv", elf], capture_output=True, check=False, timeout=600
    )


def run_rvsim(elf: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-m", "rvsim", elf, "--no-stats"],
        capture_output=True,
        check=False,
        timeout=1800,
    )


@unittest.skipUnless(os.path.exists(SPIKE), "spike not built")
@unittest.skipUnless(os.path.exists(MANDELBROT), "HTIF programs not built")
class HtifPrograms(unittest.TestCase):
    def test_a_printing_program_matches_spike(self):
        expected = run_spike(MANDELBROT)
        actual = run_rvsim(MANDELBROT)

        self.assertEqual(expected.returncode, 0, expected.stderr)
        self.assertEqual(actual.returncode, 0, actual.stderr)
        self.assertEqual(actual.stdout, expected.stdout)


if __name__ == "__main__":
    unittest.main()
