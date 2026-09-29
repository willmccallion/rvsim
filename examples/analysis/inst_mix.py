#!/usr/bin/env python3
"""Show instruction mix breakdown for each program.

Usage:
    .venv/bin/python examples/analysis/inst_mix.py
    .venv/bin/python examples/analysis/inst_mix.py --programs mandelbrot raytracer
"""

import argparse

from rvsim import Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    rows = {}
    for program in args.programs:
        binary = f"software/bin/programs/{program}.elf"
        print(f"  {program}...", flush=True)
        config = Config(uart_quiet=True)
        result = Environment(binary=binary, config=config).run(
            quiet=True, limit=args.limit
        )
        mix = result.stats.query(r"^core0\.commit\.(op|fp|vec)\.")
        rows[program] = Stats(
            {path.removeprefix("core0.commit."): n for path, n in mix.items()}
        )
    print(Stats.tabulate(rows, title="Instruction Mix"))
    print()


if __name__ == "__main__":
    main()
