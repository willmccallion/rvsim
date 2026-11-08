"""Running a little at a time: ``Simulator.step`` runs until the next
instruction retires, and ``run_until`` until a condition holds.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import subprocess
import sys
import unittest

from _programs import CODE_OFFSET, LOAD_ADDRESS, user_elf

from rvsim import Config, Simulator

# `j .`: a loop of one instruction that retires again and again.
JAL_SELF = 0x0000_006F
ENTRY = LOAD_ADDRESS + CODE_OFFSET


class Step(unittest.TestCase):
    def test_an_instruction_that_retires_twice_in_a_row_is_stepped_twice(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        first = cpu.step()
        second = cpu.step()

        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        self.assertEqual((first.pc, first.raw), (ENTRY, JAL_SELF))
        self.assertEqual((second.pc, second.raw), (ENTRY, JAL_SELF))

    def test_cycles_counts_the_cycles_the_step_ran(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))
        cpu.step()
        before = cpu.cycle

        stepped = cpu.step()

        self.assertEqual(stepped.cycles, cpu.cycle - before)


class RunUntil(unittest.TestCase):
    def test_an_unknown_privilege_is_refused_before_running(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        with self.assertRaisesRegex(ValueError, "privilege"):
            cpu.run_until(privilege="m", limit=1_000)

        self.assertEqual(cpu.cycle, 0)

    def test_a_named_privilege_stops_the_run(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        cpu.run_until(privilege="U", limit=1_000, chunk=10)

        self.assertEqual(cpu.privilege, "U")
        self.assertLess(cpu.cycle, 1_000)

    def test_a_zero_chunk_is_refused_before_running(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        with self.assertRaisesRegex(ValueError, "chunk must be at least 1"):
            cpu.run_until(pc=ENTRY + 4, limit=1_000, chunk=0)

        self.assertEqual(cpu.cycle, 0)


class Run(unittest.TestCase):
    def test_a_run_prints_nothing_of_its_own(self):
        script = (
            "from rvsim import Config, Simulator\n"
            "from _programs import user_elf\n"
            f"Simulator(Config(), elf_data=user_elf([{JAL_SELF}])).run(limit=1_000)\n"
        )

        result = subprocess.run(
            [sys.executable, "-c", script],
            cwd=os.path.dirname(os.path.abspath(__file__)),
            check=True,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.stdout, "")


class Sample(unittest.TestCase):
    def test_one_snapshot_is_taken_per_interval_up_to_the_limit(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        snapshots = cpu.sample(1_000, limit=2_500)

        self.assertEqual([s.cycles for s in snapshots], [1_000, 2_000, 2_500])

    def test_a_zero_interval_is_refused_before_running(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        with self.assertRaisesRegex(ValueError, "every must be at least 1"):
            cpu.sample(0, limit=1_000)

        self.assertEqual(cpu.cycle, 0)


if __name__ == "__main__":
    unittest.main()
