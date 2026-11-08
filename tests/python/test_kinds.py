"""What the simulator reports as a closed set comes back as an enum member
that still equals its plain string: why a run stopped and a hart's
privilege.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from _programs import EXIT_WITH_ARGC, user_elf

from rvsim import Config, Privilege, Simulator, StopReason

# `j .`: a loop of one instruction that retires again and again.
JAL_SELF = 0x0000_006F


class Kinds(unittest.TestCase):
    def test_a_run_that_reaches_its_cycles_says_so_with_a_stop_reason(self):
        sim = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        reason, value = sim.run_to(cycles=100)

        self.assertIs(reason, StopReason.CYCLES)
        self.assertEqual(reason, "cycles")
        self.assertIsNone(value)

    def test_a_run_that_exits_pairs_the_reason_with_the_exit_code(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC), args=["p", "q"])

        reason, value = sim.run_to(cycles=1_000_000)

        self.assertIs(reason, StopReason.EXIT)
        self.assertEqual(value, 2)

    def test_privilege_is_a_privilege_that_equals_its_letter(self):
        sim = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        self.assertIs(sim.privilege, Privilege.USER)
        self.assertIs(sim.harts[0].privilege, Privilege.USER)
        self.assertEqual(sim.privilege, "U")
        self.assertEqual(str(sim.privilege), "U")

    def test_run_until_takes_a_privilege_member(self):
        sim = Simulator(Config(), elf_data=user_elf([JAL_SELF]))

        sim.run_until(privilege=Privilege.USER, limit=1_000, chunk=10)

        self.assertLess(sim.cycle, 1_000)


if __name__ == "__main__":
    unittest.main()
