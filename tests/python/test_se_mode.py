"""SE mode from Python: a Linux user program runs with its arguments and
its exit code comes back.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import struct
import tempfile
import unittest

from rvsim import Config, Environment, Simulator

LOAD_ADDRESS = 0x1_0000
CODE_OFFSET = 0x80
ECALL = 0x0000_0073
SYS_EXIT = 93


def addi(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13


def ld(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (3 << 12) | (rd << 7) | 0x03


def blt(rs1: int, rs2: int, imm: int) -> int:
    imm &= 0x1FFF
    return (
        (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (4 << 12)
        | (((imm >> 1) & 0xF) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
    )


def user_elf(code: list[int]) -> bytes:
    """A static riscv64 executable with one RWX segment holding ``code``."""
    entry = LOAD_ADDRESS + CODE_OFFSET
    header = b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8)
    header += struct.pack(
        "<HHIQQQIHHHHHH", 2, 243, 1, entry, 64, 0, 0, 64, 56, 1, 0, 0, 0
    )
    size = CODE_OFFSET + 4 * len(code)
    phdr = struct.pack(
        "<IIQQQQQQ", 1, 7, 0, LOAD_ADDRESS, LOAD_ADDRESS, size, size, 0x1000
    )
    image = header + phdr
    image += bytes(CODE_OFFSET - len(image))
    image += b"".join(struct.pack("<I", word) for word in code)
    return image


# argc from the stack into a0, then exit(a0).
EXIT_WITH_ARGC = [ld(10, 2, 0), addi(17, 0, SYS_EXIT), ECALL]
# Counts to 300 in t0, then exit(9).
COUNT_THEN_EXIT = [
    addi(5, 0, 0),
    addi(6, 0, 300),
    addi(5, 5, 1),
    blt(5, 6, -4),
    addi(10, 0, 9),
    addi(17, 0, SYS_EXIT),
    ECALL,
]


class SeMode(unittest.TestCase):
    def test_a_user_program_runs_with_its_arguments(self):
        cpu = Simulator(
            Config(), elf_data=user_elf(EXIT_WITH_ARGC), args=["prog", "a", "b"]
        )

        self.assertEqual(cpu.run(limit=50_000), 3)

    def test_an_environment_passes_the_binary_as_argv0_and_its_args(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))

            result = Environment(binary=path, args=["one"]).run(limit=50_000)

        self.assertEqual(result.exit_code, 2)

    def test_the_paging_mode_is_configurable(self):
        for mode in ("Sv39", "Sv48", "Sv57"):
            cpu = Simulator(
                Config(page_mode=mode), elf_data=user_elf(EXIT_WITH_ARGC), args=["prog"]
            )
            self.assertEqual(cpu.run(limit=50_000), 1, mode)

    def test_an_unknown_paging_mode_is_refused(self):
        with self.assertRaisesRegex(ValueError, "page_mode"):
            Config(page_mode="Sv32")

    def test_a_checkpoint_resumes_a_user_program(self):
        whole = Simulator(Config(), elf_data=user_elf(COUNT_THEN_EXIT))
        expected = whole.run(limit=50_000)
        first = Simulator(Config(), elf_data=user_elf(COUNT_THEN_EXIT))
        self.assertIsNone(first.run(limit=100))

        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "point.ckpt")
            first.save(path)
            second = Simulator(Config(), elf_data=user_elf(COUNT_THEN_EXIT))
            second.restore(path)

        self.assertEqual(expected, 9)
        self.assertEqual(second.run(limit=50_000), expected)
        self.assertEqual(second.instructions_retired, whole.instructions_retired)

    def test_a_bare_metal_image_still_loads_as_one(self):
        cpu = Simulator(
            Config(), elf_data=user_elf(EXIT_WITH_ARGC), args=["prog"], mode="bare"
        )

        self.assertEqual(cpu.privilege, "M")


if __name__ == "__main__":
    unittest.main()
