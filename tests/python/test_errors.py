"""The exceptions the simulator raises: one class per kind of failure, all
of them ``SimulationError``s but a bad config.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from test_se_mode import EXIT_WITH_ARGC, user_elf

from rvsim import (
    CheckpointError,
    Config,
    ConfigError,
    KernelPanic,
    LoadError,
    SimulationError,
    Simulator,
    Stalled,
)


class Hierarchy(unittest.TestCase):
    def test_every_simulation_error_is_still_a_runtime_error(self):
        for kind in (SimulationError, LoadError, CheckpointError, KernelPanic, Stalled):
            with self.subTest(kind=kind.__name__):
                self.assertTrue(issubclass(kind, SimulationError))
                self.assertTrue(issubclass(kind, RuntimeError))

    def test_a_config_error_is_a_value_error(self):
        self.assertTrue(issubclass(ConfigError, ValueError))
        self.assertFalse(issubclass(ConfigError, SimulationError))


class Raised(unittest.TestCase):
    def test_a_config_field_of_the_wrong_type_is_a_config_error(self):
        with self.assertRaisesRegex(ConfigError, "Invalid config"):
            Simulator({"general": {"trace_instructions": "yes"}})

    def test_bytes_that_are_not_an_elf_are_a_load_error(self):
        with self.assertRaisesRegex(LoadError, "not an ELF"):
            Simulator(Config(), elf_data=b"not an elf at all", mode="se")

    def test_restoring_a_file_that_is_not_a_checkpoint_is_a_checkpoint_error(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "junk.ckpt")
            with open(path, "wb") as f:
                f.write(b"{}\n")

            with self.assertRaises(CheckpointError):
                sim.restore(path)

    def test_restoring_a_missing_file_is_a_file_not_found_error(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))

        with self.assertRaisesRegex(
            FileNotFoundError, "cannot open checkpoint file /no/such"
        ):
            sim.restore("/no/such")

    def test_run_until_with_no_condition_is_a_type_error(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))

        with self.assertRaisesRegex(TypeError, "requires at least one"):
            sim.run_until(limit=10)


if __name__ == "__main__":
    unittest.main()
