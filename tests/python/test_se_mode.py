"""SE mode from Python: a Linux user program runs with its arguments and
its exit code comes back.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from _programs import (
    COUNT_THEN_EXIT,
    ECALL,
    EXIT_WITH_ARGC,
    M5_FAIL,
    SCRIPT,
    SYS_EXIT,
    SYS_WRITE,
    addi,
    auipc,
    beq,
    file_exchange,
    lui,
    lw,
    sub,
    sw,
    user_elf,
)

from rvsim import Config, Environment, LoadError, Program, Simulator

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
SYS_CLONE = 220
SYS_EXIT_GROUP = 94
S0, T0, T1, A0, A1, A7, SP = 8, 5, 6, 10, 11, 17, 2
# clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD,
# sp - 64 KiB): a0 = 0x10f00, a1 = sp - 0x10000.
CLONE_A_THREAD = [
    lui(A0, 0x11),
    addi(A0, A0, -0x100),
    lui(T0, 0x10),
    sub(A1, SP, T0),
    addi(A7, 0, SYS_CLONE),
    ECALL,
]
# The thread stores 42 in a word of the program; the first thread waits
# for it and ends the process with it.
THREAD_SHARES_MEMORY = [
    auipc(S0, 0),
    addi(S0, S0, 18 * 4),
    *CLONE_A_THREAD,
    beq(A0, 0, 5 * 4),
    lw(A0, S0, 0),
    beq(A0, 0, -4),
    addi(A7, 0, SYS_EXIT_GROUP),
    ECALL,
    addi(T1, 0, 42),
    sw(T1, S0, 0),
    addi(A0, 0, 0),
    addi(A7, 0, SYS_EXIT),
    ECALL,
    0,
]
# clone with no free hart, then exit(-a0).
CLONE_WITHOUT_A_HART = [*CLONE_A_THREAD, sub(A0, 0, A0), addi(A7, 0, SYS_EXIT), ECALL]
EAGAIN = 11


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
        cpu = Simulator(Config(console="Captured"), elf_data=user_elf(WRITE_HI))

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

    def test_an_unknown_load_mode_is_refused(self):
        with self.assertRaisesRegex(ValueError, "mode must be 'se' or 'bare'"):
            Simulator(Config(), elf_data=user_elf(EXIT_WITH_ARGC), mode="kernel")


class Threads(unittest.TestCase):
    """A program's threads run on the harts its first thread leaves free,
    one thread per hart, sharing its memory."""

    def test_a_thread_runs_on_a_second_hart_and_shares_memory(self):
        cpu = Simulator(Config(hart_count=2), elf_data=user_elf(THREAD_SHARES_MEMORY))

        exit_code = cpu.run(limit=200_000)

        self.assertEqual(exit_code, 42)
        self.assertEqual(cpu.exit_codes, [42])
        self.assertGreater(cpu.harts[1].instructions_retired, 0)

    def test_clone_fails_with_eagain_when_no_hart_is_free(self):
        cpu = Simulator(Config(), elf_data=user_elf(CLONE_WITHOUT_A_HART))

        self.assertEqual(cpu.run(limit=200_000), EAGAIN)


class SeveralPrograms(unittest.TestCase):
    """Several programs run at once, one per hart, and the run ends when the
    last of them exits."""

    def test_programs_and_a_binary_together_are_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))
            with self.assertRaisesRegex(ValueError, "programs or a binary, not both"):
                Simulator(
                    Config(),
                    elf_data=user_elf(EXIT_WITH_ARGC),
                    programs=[Program(path)],
                )

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
            with self.assertRaisesRegex(LoadError, "hart_count"):
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
        with self.assertRaisesRegex(LoadError, "could not read file"):
            Simulator(Config(), elf_data=user_elf(file_exchange()), script="/no/such")


if __name__ == "__main__":
    unittest.main()
