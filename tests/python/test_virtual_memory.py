"""Reads through a hart's page tables: ``sim.vmem32`` uses hart 0's, and
``sim.harts[n].vmem32`` hart ``n``'s.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from _programs import CODE_OFFSET, EXIT_WITH_ARGC, LOAD_ADDRESS, user_elf

from rvsim import Config, Program, Simulator

# `j .`: a loop of one instruction.
JAL_SELF = 0x0000_006F
ENTRY = LOAD_ADDRESS + CODE_OFFSET


def two_processes() -> Simulator:
    """Two harts, each running its own program mapped at ``ENTRY``."""
    with tempfile.TemporaryDirectory() as tmp:
        paths = []
        for name, code in (("loop", [JAL_SELF]), ("exit", EXIT_WITH_ARGC)):
            path = os.path.join(tmp, f"{name}.elf")
            with open(path, "wb") as f:
                f.write(user_elf(code))
            paths.append(path)
        return Simulator(Config(hart_count=2), programs=[Program(p) for p in paths])


class VirtualMemory(unittest.TestCase):
    def test_each_hart_reads_through_its_own_page_tables(self):
        sim = two_processes()

        self.assertEqual(sim.harts[0].vmem32[ENTRY], JAL_SELF)
        self.assertNotEqual(sim.harts[1].vmem32[ENTRY], JAL_SELF)

    def test_the_simulator_view_reads_through_hart_0(self):
        sim = two_processes()

        self.assertEqual(sim.vmem32[ENTRY], sim.harts[0].vmem32[ENTRY])
        self.assertEqual(sim.vmem64[ENTRY], sim.harts[0].vmem64[ENTRY])

    def test_an_unmapped_address_raises_value_error(self):
        sim = two_processes()

        with self.assertRaisesRegex(ValueError, "translation failed"):
            sim.harts[1].vmem32[0x10]


if __name__ == "__main__":
    unittest.main()
