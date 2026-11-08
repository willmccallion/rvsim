"""Checks an installed rvsim: it imports, reports the version it was built
as, and runs a program to the exit code the program chooses.

Run it isolated, so that ``import rvsim`` finds the installed package and
not the source tree::

    python -I tools/wheel_smoke.py 3.0.0
"""

import pathlib
import sys

sys.path.insert(
    0, str(pathlib.Path(__file__).resolve().parents[1] / "tests" / "python")
)

from _programs import EXIT_WITH_ARGC, user_elf

import rvsim

# EXIT_WITH_ARGC exits with its argument count.
ARGS = ["prog", "one", "two"]
CYCLE_LIMIT = 100_000


def main() -> int:
    expected = sys.argv[1]
    if rvsim.__version__ != expected:
        print(
            f"rvsim reports {rvsim.__version__}, expected {expected}", file=sys.stderr
        )
        return 1
    sim = rvsim.Simulator(rvsim.Config(), elf_data=user_elf(EXIT_WITH_ARGC), args=ARGS)
    exit_code = sim.run(limit=CYCLE_LIMIT)
    if exit_code != len(ARGS):
        print(
            f"the program exited with {exit_code}, expected {len(ARGS)}",
            file=sys.stderr,
        )
        return 1
    print(f"rvsim {rvsim.__version__} imports and runs a program")
    return 0


if __name__ == "__main__":
    sys.exit(main())
