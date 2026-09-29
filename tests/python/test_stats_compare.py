"""Tests for how comparisons classify and aggregate stat paths.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import contextlib
import io
import unittest

from rvsim import Result, Stats
from rvsim.stats import _aggregate_rate, _better, _Better


def run(**stats):
    return Stats({k.replace("__", "."): v for k, v in stats.items()})


class Direction(unittest.TestCase):
    def test_ipc_and_accuracy_are_better_higher(self):
        self.assertIs(_better("ipc"), _Better.HIGHER)
        self.assertIs(_better("core0.bp.committed.accuracy"), _Better.HIGHER)

    def test_cycles_misses_and_miss_rates_are_better_lower(self):
        self.assertIs(_better("cycles"), _Better.LOWER)
        self.assertIs(_better("core0.cache.l1d.misses"), _Better.LOWER)
        self.assertIs(_better("llc.miss_rate"), _Better.LOWER)

    def test_every_stall_counter_is_better_lower(self):
        self.assertIs(_better("core0.pipeline.stalls.data"), _Better.LOWER)

    def test_a_counter_without_a_direction_has_none(self):
        self.assertIsNone(_better("core0.commit.op.load"))


class RateAggregation(unittest.TestCase):
    def test_accuracy_is_recomputed_from_the_summed_counters(self):
        runs = [
            run(core0__bp__hits=90, core0__bp__mispredicts=10, core0__bp__accuracy=0.9),
            run(core0__bp__hits=10, core0__bp__mispredicts=90, core0__bp__accuracy=0.1),
        ]

        aggregate = _aggregate_rate("core0.bp.accuracy", runs)

        self.assertAlmostEqual(aggregate, 0.5)

    def test_miss_rate_is_misses_over_all_accesses(self):
        runs = [
            run(llc__hits=30, llc__misses=10, llc__miss_rate=0.25),
            run(llc__hits=50, llc__misses=10, llc__miss_rate=1 / 6),
        ]

        aggregate = _aggregate_rate("llc.miss_rate", runs)

        self.assertAlmostEqual(aggregate, 0.2)

    def test_ipc_is_total_instructions_over_total_cycles(self):
        runs = [
            run(ipc=2.0, instructions_retired=200, cycles=100),
            run(ipc=0.5, instructions_retired=100, cycles=200),
        ]

        aggregate = _aggregate_rate("ipc", runs)

        self.assertAlmostEqual(aggregate, 300 / 300)

    def test_a_missing_counter_gives_no_aggregate(self):
        runs = [run(core0__bp__accuracy=0.9)]

        self.assertIsNone(_aggregate_rate("core0.bp.accuracy", runs))


class DefaultComparison(unittest.TestCase):
    def test_shows_the_headline_paths_when_no_metrics_are_named(self):
        stats = run(
            cycles=100,
            ipc=1.0,
            core0__bp__committed__accuracy=0.9,
            core0__cache__l1d__miss_rate=0.1,
            core0__commit__op__load=7,
        )
        results = {"a": Result(exit_code=0, stats=stats)}

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            Result.compare(results)

        table = out.getvalue()
        self.assertIn("core0.bp.committed.accuracy", table)
        self.assertIn("core0.cache.l1d.miss_rate", table)
        self.assertNotIn("core0.commit.op.load", table)


if __name__ == "__main__":
    unittest.main()
