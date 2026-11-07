"""The stall limit: a run fails when a hart retires nothing for that many
cycles outside WFI, and None turns the check off.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import unittest

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")
DEFAULT_STALL_LIMIT = 1_000_000
# Fewer cycles than the first instruction takes to fetch from cold caches.
SHORTER_THAN_A_COLD_FETCH = 10


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class StallLimit(unittest.TestCase):
    def test_it_defaults_to_a_million_cycles(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)

        self.assertEqual(sim.stall_limit, DEFAULT_STALL_LIMIT)

    def test_a_hart_stalled_past_the_limit_fails_the_run_naming_it(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)
        sim.stall_limit = SHORTER_THAN_A_COLD_FETCH

        with self.assertRaisesRegex(
            RuntimeError, "hart 0 retired nothing for 10 cycles"
        ):
            sim.run()

    def test_none_turns_the_check_off(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)
        sim.stall_limit = None

        self.assertEqual(sim.run(), 0)
        self.assertIsNone(sim.stall_limit)


if __name__ == "__main__":
    unittest.main()
