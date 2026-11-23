"""Python API tests for the core's hierarchical stats query.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from _programs import bundled, needs

from rvsim import Config, Simulator

FIB = bundled("programs", "fib")


@needs(FIB)
class QueryResultIteration(unittest.TestCase):
    def test_iterating_yields_every_path_value_pair(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)
        sim.run()
        result = sim.stats.query("**")

        pairs = list(result)

        self.assertEqual(len(pairs), len(result))
        self.assertEqual([path for path, _ in pairs], result.paths())
        self.assertAlmostEqual(sum(value for _, value in pairs), result.sum())


PENALTY = "core0.bp.mispredict_penalty"


@needs(FIB)
class HistogramLookup(unittest.TestCase):
    def test_a_recorded_histogram_reports_its_samples(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)
        sim.run()

        penalties = sim.stats.histogram(PENALTY)

        self.assertIsNotNone(penalties)
        self.assertGreater(penalties.count, 0)
        self.assertLessEqual(penalties.min, penalties.mean)
        self.assertLessEqual(penalties.mean, penalties.max)
        self.assertAlmostEqual(penalties.mean, penalties.sum / penalties.count)

    def test_a_region_knows_its_samples_but_not_their_extremes(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)
        sim.run(limit=20_000)
        earlier = sim.stats
        sim.run()

        region = (sim.stats - earlier).histogram(PENALTY)

        self.assertEqual(
            region.count,
            sim.stats.histogram(PENALTY).count - earlier.histogram(PENALTY).count,
        )
        self.assertIsNone(region.min)
        self.assertIsNone(region.max)

    def test_a_path_with_nothing_recorded_has_no_histogram(self):
        sim = Simulator(Config(console="Quiet"), binary=FIB)

        self.assertIsNone(sim.stats.histogram(PENALTY))


if __name__ == "__main__":
    unittest.main()
