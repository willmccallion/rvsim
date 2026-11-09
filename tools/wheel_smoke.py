"""Checks an installed rvsim: it imports, reports the version it was built
as, and runs a program to the exit code the program chooses: a user
program under SE mode, or a bare-metal one where SE mode needs a Unix host
that this is not.

Run it isolated, so that ``import rvsim`` finds the installed package and
not the source tree::

    python -I tools/wheel_smoke.py 3.0.0
"""

import os
import pathlib
import sys

sys.path.insert(
    0, str(pathlib.Path(__file__).resolve().parents[1] / "tests" / "python")
)

from _programs import EXIT_WITH_ARGC, M5_FAIL, addi, bare_metal_elf, user_elf

import rvsim

# EXIT_WITH_ARGC exits with its argument count.
ARGS = ["prog", "one", "two"]
BARE_METAL_EXIT_CODE = 7
# m5_fail with a1 = BARE_METAL_EXIT_CODE.
EXIT_WITH_SEVEN = [addi(11, 0, BARE_METAL_EXIT_CODE), 0x7B | (M5_FAIL << 25)]
CYCLE_LIMIT = 100_000


def run_program() -> tuple[int, int]:
    """Runs the program this host can and returns its exit code and the one
    it should have."""
    if os.name == "posix":
        elf = user_elf(EXIT_WITH_ARGC)
        sim = rvsim.Simulator(rvsim.Config(), elf_data=elf, args=ARGS)
        return sim.run(limit=CYCLE_LIMIT), len(ARGS)
    sim = rvsim.Simulator(rvsim.Config(), elf_data=bare_metal_elf(EXIT_WITH_SEVEN))
    return sim.run(limit=CYCLE_LIMIT), BARE_METAL_EXIT_CODE


def main() -> int:
    expected = sys.argv[1]
    if rvsim.__version__ != expected:
        print(
            f"rvsim reports {rvsim.__version__}, expected {expected}", file=sys.stderr
        )
        return 1
    exit_code, expected_code = run_program()
    if exit_code != expected_code:
        print(
            f"the program exited with {exit_code}, expected {expected_code}",
            file=sys.stderr,
        )
        return 1
    print(f"rvsim {rvsim.__version__} imports and runs a program")
    return 0


if __name__ == "__main__":
    sys.exit(main())
