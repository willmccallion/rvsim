"""Python API tests that run the built simulator on a real program.

Run with: .venv/bin/python -m unittest discover -s testing/python
"""

import os
import unittest

from rvsim import Backend, Config, Environment, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class EnvironmentRun(unittest.TestCase):
    def test_run_returns_exit_zero_and_stats(self):
        result = Environment(binary=FIB, config=Config(uart_quiet=True)).run(
            quiet=False
        )

        self.assertTrue(result.ok, result.stats)
        self.assertGreater(result.stats["cycles"], 0)
        self.assertGreater(result.stats["instructions_retired"], 0)
        self.assertIn("core0.cache.l1d.hits", result.stats)
        self.assertEqual(
            len(result.stats.query("l1d")), len(result.stats.query("cache\\.l1d"))
        )

    def test_run_accepts_a_config_dict(self):
        config = Config(width=2, backend=Backend.InOrder(), uart_quiet=True).to_dict()

        result = Environment(binary=FIB, config=config).run(quiet=False)

        self.assertTrue(result.ok)

    def test_simulator_wrapper_runs_the_same_binary(self):
        sim = Simulator(Config(uart_quiet=True), binary=FIB)

        exit_code = sim.run()

        self.assertEqual(exit_code, 0)
        self.assertGreater(sim.stats.cycles, 0)


if __name__ == "__main__":
    unittest.main()
