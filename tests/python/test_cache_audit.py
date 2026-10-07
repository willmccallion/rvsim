"""The cache audit: off by default, and when switched on a run with every
cache invariant intact ends as it would without it.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import unittest

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class CacheAudit(unittest.TestCase):
    def test_it_is_off_unless_switched_on(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)

        self.assertFalse(sim.audit_caches)

    def test_an_audited_run_ends_as_an_unaudited_one(self):
        plain = Simulator(Config(uart_quiet=True), binary=FIB)
        audited = Simulator(Config(uart_quiet=True), binary=FIB)
        audited.audit_caches = True

        codes = [sim.run(stats_sections=None) for sim in (plain, audited)]

        self.assertEqual(codes, [0, 0])
        self.assertEqual(audited.stats.cycles, plain.stats.cycles)


if __name__ == "__main__":
    unittest.main()
