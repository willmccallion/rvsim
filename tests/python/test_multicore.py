"""The bundled multi-core programs run on several harts: every hart starts
at the entry with its id, finds the hart count in the device tree, prints
through HTIF without the harts' output tearing, and hart 0's return value
ends the run.
"""

import os
import unittest

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SPINLOCK = os.path.join(ROOT, "software", "bin", "multicore", "spinlock.elf")


@unittest.skipUnless(os.path.exists(SPINLOCK), "spinlock.elf not built")
class MulticorePrograms(unittest.TestCase):
    def test_spinlock_counts_on_every_hart(self):
        cpu = Simulator(Config(hart_count=4, console="Captured"), binary=SPINLOCK)

        code = cpu.run(limit=50_000_000)

        self.assertEqual(code, 0)
        self.assertIn(
            "spinlock: 4 harts, counter=8000 expected=8000", cpu.read_console()
        )


if __name__ == "__main__":
    unittest.main()
