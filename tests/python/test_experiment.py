"""Python API tests that run the built simulator on a real program.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from _programs import bundled, needs

from rvsim import Backend, Config, Environment, MemoryController, Simulator

FIB = bundled("programs", "fib")


@needs(FIB)
class EnvironmentRun(unittest.TestCase):
    def test_run_returns_exit_zero_and_stats(self):
        result = Environment(binary=FIB, config=Config(console="Quiet")).run(
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
        config = Config(width=2, backend=Backend.InOrder(), console="Quiet").to_dict()

        result = Environment(binary=FIB, config=config).run(quiet=False)

        self.assertTrue(result.ok)

    def test_simulator_wrapper_runs_the_same_binary(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)

        exit_code = sim.run()

        self.assertEqual(exit_code, 0)
        self.assertGreater(sim.stats.cycles, 0)


@needs(FIB)
class SimpleControllerLatency(unittest.TestCase):
    def cycles_with_memory_latency(self, latency):
        config = Config(
            console="Quiet",
            memory_controller=MemoryController.Simple(latency=latency),
        )
        return Environment(binary=FIB, config=config).run().stats["cycles"]

    def test_a_longer_latency_slows_the_run(self):
        fast = self.cycles_with_memory_latency(120)

        slow = self.cycles_with_memory_latency(400)

        self.assertGreater(slow, fast)


if __name__ == "__main__":
    unittest.main()
