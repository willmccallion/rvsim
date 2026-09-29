"""Python API tests for the core's hierarchical stats query.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import unittest

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class QueryResultIteration(unittest.TestCase):
    def test_iterating_yields_every_path_value_pair(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)
        sim.run()
        result = sim.stats.query("**")

        pairs = list(result)

        self.assertEqual(len(pairs), len(result))
        self.assertEqual([path for path, _ in pairs], result.paths())
        self.assertAlmostEqual(sum(value for _, value in pairs), result.sum())


if __name__ == "__main__":
    unittest.main()
