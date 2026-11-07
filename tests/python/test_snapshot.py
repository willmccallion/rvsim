"""``Simulator.pipeline_snapshot`` shows what each latch holds.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from test_se_mode import user_elf

from rvsim import Config, Simulator

# `j .`, so fetch always has something in flight.
JAL_SELF = 0x0000_006F
# Long enough for the first fetch to miss in the cold caches and return.
FIRST_FETCH_CYCLES = 5_000


class Snapshot(unittest.TestCase):
    def test_a_fetch_slot_holds_an_address_but_no_instruction_yet(self):
        cpu = Simulator(Config(), elf_data=user_elf([JAL_SELF]))
        slots = []
        while not slots and cpu.cycle < FIRST_FETCH_CYCLES:
            cpu.tick()
            slots = cpu.pipeline_snapshot().fetch1_fetch2

        self.assertTrue(slots, "fetch had an address in flight")
        self.assertEqual(set(slots[0]), {"pc", "pred_taken", "pred_target"})


if __name__ == "__main__":
    unittest.main()
