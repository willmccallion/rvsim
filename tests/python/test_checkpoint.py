"""Checkpoints on every preset and both backends.

A checkpoint carries the architectural state; caches, TLBs and predictors
restart cold after a restore. So a run split by a checkpoint must end
exactly where an uninterrupted one does, restoring the same checkpoint must
always run the same, and a restored checkpoint must save back to the same
bytes.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from rvsim import Backend, Simulator, presets

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


def configs():
    """Every preset, on its own backend and on the in-order one."""
    for name, preset in presets.PRESETS.items():
        config = preset().replace(console="captured")
        yield name, config
        yield f"{name} in-order", config.replace(backend=Backend.InOrder())


def boot(config):
    return Simulator(config, binary=FIB)


def finish(sim):
    """Runs to the program's exit; returns what it leaves behind."""
    exit_code = sim.run(stats_sections=None)
    return exit_code, sim.instructions_retired


def save(sim, directory, name):
    path = os.path.join(directory, name)
    sim.save(path)
    return path


def restored(config, path):
    sim = boot(config)
    sim.restore(path)
    return sim


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class Checkpoint(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.dir = directory.name

    def test_a_run_split_by_a_checkpoint_ends_as_an_uninterrupted_one(self):
        for name, config in configs():
            with self.subTest(config=name):
                whole = boot(config)
                expected = finish(whole)
                expected_console = whole.read_console()
                first = boot(config)
                first.run(limit=whole.cycle // 2, stats_sections=None)
                path = save(first, self.dir, f"{name}.ckpt")
                console_before = first.read_console()

                second = restored(config, path)
                outcome = finish(second)

                self.assertEqual(outcome, expected)
                self.assertEqual(
                    console_before + second.read_console(), expected_console
                )

    def test_restoring_a_checkpoint_twice_runs_the_same_cycles(self):
        for name, config in configs():
            with self.subTest(config=name):
                sim = boot(config)
                sim.run(limit=20_000, stats_sections=None)
                path = save(sim, self.dir, f"{name}.ckpt")

                runs = []
                for _ in range(2):
                    again = restored(config, path)
                    finish(again)
                    runs.append((again.cycle, again.instructions_retired))

                self.assertEqual(runs[0], runs[1])

    def test_a_restored_checkpoint_saves_the_same_bytes(self):
        for name, config in configs():
            with self.subTest(config=name):
                sim = boot(config)
                sim.run(limit=20_000, stats_sections=None)
                original = save(sim, self.dir, f"{name}.ckpt")

                resaved = save(
                    restored(config, original), self.dir, f"{name}-again.ckpt"
                )

                with open(original, "rb") as a, open(resaved, "rb") as b:
                    self.assertEqual(a.read(), b.read())

    def test_a_checkpoint_from_one_backend_finishes_on_the_other(self):
        for name, preset in presets.PRESETS.items():
            with self.subTest(preset=name):
                o3 = preset().replace(console="captured")
                in_order = o3.replace(backend=Backend.InOrder())
                whole = boot(in_order)
                expected = finish(whole)
                first = boot(o3)
                first.run(limit=20_000, stats_sections=None)
                path = save(first, self.dir, f"{name}.ckpt")

                outcome = finish(restored(in_order, path))

                self.assertEqual(outcome, expected)


if __name__ == "__main__":
    unittest.main()
