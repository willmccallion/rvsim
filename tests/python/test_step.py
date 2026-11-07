"""Running a little at a time: ``Simulator.step`` runs until the next
instruction retires, and ``run_until`` until a condition holds.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from test_se_mode import CODE_OFFSET, LOAD_ADDRESS, user_elf

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


if __name__ == "__main__":
    unittest.main()


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
