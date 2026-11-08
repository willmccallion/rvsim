"""The cache audit: off by default, and when switched on a run with every
cache invariant intact ends as it would without it.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from _programs import bundled, needs

from rvsim import Config, Simulator

FIB = bundled("programs", "fib")


@needs(FIB)
class CacheAudit(unittest.TestCase):
    def test_it_is_off_unless_switched_on(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)

        self.assertFalse(sim.audit_caches)

    def test_an_audited_run_ends_as_an_unaudited_one(self):
        plain = Simulator(Config(console="Quiet"), binary=FIB)
        audited = Simulator(Config(console="Quiet"), binary=FIB)
        audited.audit_caches = True

        codes = [sim.run() for sim in (plain, audited)]

        self.assertEqual(codes, [0, 0])
        self.assertEqual(audited.stats.cycles, plain.stats.cycles)


if __name__ == "__main__":
    unittest.main()
