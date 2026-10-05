"""Every cache's prefetch accounting on every preset: each issued prefetch
ends at most once, as late, useful or unused. That holds at every point
of a run, so each preset runs a bounded slice of qsort.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import unittest

from rvsim import Simulator, presets

CYCLES = 300_000

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
QSORT = os.path.join(ROOT, "software", "bin", "programs", "qsort.elf")


def prefetch_counts(stats):
    """`{cache: {stat: value}}` for every cache's prefetch stats."""
    caches = {}
    for path, value in stats.query("**.prefetches.*"):
        cache, _, stat = path.rpartition(".prefetches.")
        caches.setdefault(cache, {})[stat] = value
    return caches


@unittest.skipUnless(os.path.exists(QSORT), "qsort.elf not built")
class PrefetchAccounting(unittest.TestCase):
    def test_no_cache_resolves_more_prefetches_than_it_issued(self):
        for name, preset in presets.PRESETS.items():
            sim = Simulator(preset().replace(uart_quiet=True), binary=QSORT)
            sim.run(limit=CYCLES, stats_sections=None)

            caches = prefetch_counts(sim.stats)

            issued = sum(counts["issued"] for counts in caches.values())
            self.assertGreater(issued, 0, name)
            for cache, counts in caches.items():
                with self.subTest(preset=name, cache=cache):
                    resolved = counts["late"] + counts["useful"] + counts["unused"]
                    self.assertLessEqual(resolved, counts["issued"])
                    self.assertEqual(counts["used"], counts["late"] + counts["useful"])
                    self.assertGreaterEqual(counts["accuracy"], 0.0)
                    self.assertLessEqual(counts["accuracy"], 1.0)


if __name__ == "__main__":
    unittest.main()
