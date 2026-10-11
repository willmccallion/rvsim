"""SE mode from Python: a Linux user program runs with its arguments and
its exit code comes back.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import struct
import tempfile
import unittest

from rvsim import Config, Environment, Program, Simulator

LOAD_ADDRESS = 0x1_0000
CODE_OFFSET = 0x80
ECALL = 0x0000_0073
SYS_EXIT = 93
SYS_WRITE = 64
M5_FAIL = 0x22


def addi(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13


def ld(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (3 << 12) | (rd << 7) | 0x03


def auipc(rd: int, imm: int) -> int:
    return ((imm & 0xFFFFF) << 12) | (rd << 7) | 0x17


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
# write(1, "hi\n", 3), then exit with what write returned.
WRITE_HI = [
    auipc(11, 0),
    addi(11, 11, 8 * 4),
    addi(10, 0, 1),
    addi(12, 0, 3),
    addi(17, 0, SYS_WRITE),
    ECALL,
    addi(17, 0, SYS_EXIT),
    ECALL,
    int.from_bytes(b"hi\n\0", "little"),
]
# gem5's m5_fail(0, 7): the pseudo-instruction ends the run with code 7.
M5_FAIL_7 = [addi(11, 0, 7), 0x7B | (M5_FAIL << 25)]
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

    def test_a_pseudo_op_ends_the_program_with_its_code(self):
        cpu = Simulator(Config(), elf_data=user_elf(M5_FAIL_7))

        self.assertEqual(cpu.run(limit=50_000), 7)

    def test_a_captured_console_holds_what_the_program_prints(self):
        cpu = Simulator(Config(console="captured"), elf_data=user_elf(WRITE_HI))

        self.assertEqual(cpu.run(limit=50_000), 3)
        self.assertEqual(cpu.read_console(), "hi\n")

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


M5_READ_FILE = 0x50
M5_WRITE_FILE = 0x4F
BUFFER_OFFSET = 0x100
NAME_OFFSET = 0x180
SCRIPT = b"echo from the host\n"


def file_exchange() -> list[int]:
    """Reads the script into a buffer past the code, writes the buffer to
    ``out.txt`` in the output directory, and exits with the bytes written."""
    code = [
        auipc(10, 0),
        addi(10, 10, BUFFER_OFFSET),
        addi(11, 0, 64),
        addi(12, 0, 0),
        0x7B | (M5_READ_FILE << 25),
        addi(5, 10, 0),
        auipc(10, 0),
        addi(10, 10, BUFFER_OFFSET - 6 * 4),
        addi(11, 5, 0),
        addi(12, 0, 0),
        auipc(13, 0),
        addi(13, 13, NAME_OFFSET - 10 * 4),
        0x7B | (M5_WRITE_FILE << 25),
        addi(11, 10, 0),
        0x7B | (M5_FAIL << 25),
    ]
    code += [0] * (NAME_OFFSET // 4 - len(code))
    name = b"out.txt\0"
    code += [int.from_bytes(name[i : i + 4], "little") for i in range(0, 8, 4)]
    return code


class SeveralPrograms(unittest.TestCase):
    """Several programs run at once, one per hart, and the run ends when the
    last of them exits."""

    def test_two_programs_run_on_two_harts(self):
        with tempfile.TemporaryDirectory() as tmp:
            quick = os.path.join(tmp, "quick.elf")
            with open(quick, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))
            slow = os.path.join(tmp, "slow.elf")
            with open(slow, "wb") as f:
                f.write(user_elf(COUNT_THEN_EXIT))
            cpu = Simulator(
                Config(hart_count=2),
                programs=[Program(slow), Program(quick, args=["one"])],
            )

            exit_code = cpu.run(limit=200_000)

        self.assertEqual(exit_code, 9)
        self.assertEqual(cpu.exit_codes, [9, 2])
        self.assertGreater(
            cpu.harts[0].instructions_retired, cpu.harts[1].instructions_retired
        )

    def test_exit_codes_hold_one_entry_per_program(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))
            cpu = Simulator(
                Config(hart_count=3),
                programs=[Program(path), Program(path, args=["one", "two"])],
            )

            exit_code = cpu.run(limit=200_000)

        self.assertEqual(exit_code, 1)
        self.assertEqual(cpu.exit_codes, [1, 3])

    def test_a_pseudo_op_exit_ends_the_whole_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            quick = os.path.join(tmp, "quick.elf")
            with open(quick, "wb") as f:
                f.write(user_elf(M5_FAIL_7))
            slow = os.path.join(tmp, "slow.elf")
            with open(slow, "wb") as f:
                f.write(user_elf(COUNT_THEN_EXIT))
            cpu = Simulator(
                Config(hart_count=2), programs=[Program(slow), Program(quick)]
            )

            exit_code = cpu.run(limit=200_000)

        self.assertEqual(exit_code, 7)
        self.assertEqual(cpu.exit_codes, [None, None])

    def test_more_programs_than_harts_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))
            with self.assertRaisesRegex(RuntimeError, "hart_count"):
                Simulator(Config(), programs=[Program(path), Program(path)])


class HostFiles(unittest.TestCase):
    """gem5's readfile and writefile pseudo-operations reach the script
    file and output directory the Simulator was given."""

    def test_a_program_reads_the_script_and_writes_a_file_to_the_outdir(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = os.path.join(tmp, "script.sh")
            with open(script, "wb") as f:
                f.write(SCRIPT)
            outdir = os.path.join(tmp, "m5out")
            cpu = Simulator(
                Config(),
                elf_data=user_elf(file_exchange()),
                script=script,
                outdir=outdir,
            )

            exit_code = cpu.run(limit=50_000)

            with open(os.path.join(outdir, "out.txt"), "rb") as f:
                written = f.read()
        self.assertEqual(exit_code, len(SCRIPT))
        self.assertEqual(written, SCRIPT)

    def test_an_environment_passes_the_script_and_the_outdir(self):
        with tempfile.TemporaryDirectory() as tmp:
            script = os.path.join(tmp, "script.sh")
            with open(script, "wb") as f:
                f.write(SCRIPT)
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(file_exchange()))
            outdir = os.path.join(tmp, "m5out")

            result = Environment(binary=path, script=script, outdir=outdir).run(
                limit=50_000
            )

            with open(os.path.join(outdir, "out.txt"), "rb") as f:
                written = f.read()
        self.assertEqual(result.exit_code, len(SCRIPT))
        self.assertEqual(written, SCRIPT)

    def test_without_a_script_readfile_reads_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            cpu = Simulator(Config(), elf_data=user_elf(file_exchange()), outdir=tmp)

            exit_code = cpu.run(limit=50_000)

            with open(os.path.join(tmp, "out.txt"), "rb") as f:
                written = f.read()
        self.assertEqual(exit_code, 0)
        self.assertEqual(written, b"")

    def test_a_missing_script_file_is_an_error(self):
        with self.assertRaisesRegex(RuntimeError, "could not read file"):
            Simulator(Config(), elf_data=user_elf(file_exchange()), script="/no/such")


if __name__ == "__main__":
    unittest.main()
