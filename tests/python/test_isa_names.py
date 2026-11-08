"""Register and CSR names come from the simulator's own tables, so every
CSR it implements can be named from Python.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from _programs import EXIT_WITH_ARGC, user_elf

from rvsim import Config, Simulator, csr, reg


class Registers(unittest.TestCase):
    def test_abi_names_and_aliases_name_their_index(self):
        self.assertEqual((reg.RA, reg.A0, reg.T6), (1, 10, 31))
        self.assertEqual((reg("fp"), reg("s0"), reg("x5")), (8, 8, 5))
        self.assertEqual(reg.name(10), "a0")

    def test_an_unknown_register_attribute_is_an_attribute_error(self):
        with self.assertRaises(AttributeError):
            _ = reg.Q9


class Csrs(unittest.TestCase):
    def test_every_implemented_csr_has_a_name(self):
        self.assertEqual(csr.MSTATUS, 0x300)
        self.assertEqual(csr.MCOUNTINHIBIT, 0x320)
        self.assertEqual(csr.VLENB, 0xC22)
        self.assertEqual(csr.name(0x320), "mcountinhibit")

    def test_a_hart_reads_a_csr_by_name_or_address(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))

        self.assertEqual(sim.csrs["vlenb"], sim.csrs[csr.VLENB])
        self.assertEqual(sim.csrs["MHARTID"], 0)

    def test_an_address_beyond_twelve_bits_is_refused(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))

        with self.assertRaisesRegex(KeyError, "12-bit"):
            sim.csrs[0x1300]

    def test_an_unknown_name_is_refused(self):
        sim = Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC))

        with self.assertRaisesRegex(KeyError, "unknown CSR name"):
            sim.csrs["mfoo"]


if __name__ == "__main__":
    unittest.main()
