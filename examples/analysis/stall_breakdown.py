#!/usr/bin/env python3
"""Show which pipeline stalls dominate across predictors and widths.

Usage:
    .venv/bin/python examples/analysis/stall_breakdown.py
    .venv/bin/python examples/analysis/stall_breakdown.py --widths 1 2 4
"""

import argparse

from rvsim import BranchPredictor, Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
BP_MAP = {
    "Static": BranchPredictor.Static,
    "TAGE": BranchPredictor.TAGE,
}


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--widths", type=int, nargs="+", default=[1, 2, 4], help="Pipeline widths"
    )
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    for program in args.programs:
        binary = f"software/bin/programs/{program}.elf"
        rows = {}
        for bp_name, bp_cls in BP_MAP.items():
            for width in args.widths:
                label = f"{bp_name}/w{width}"
                print(f"  {program} {label}...", flush=True)
                config = Config(branch_predictor=bp_cls(), uart_quiet=True, width=width)
                result = Environment(binary=binary, config=config).run(
                    quiet=True, limit=args.limit
                )
                s = result.stats
                stalls = s.query(r"^core0\.pipeline\.stalls\.")
                rows[label] = Stats(
                    {
                        "cycles": s["cycles"],
                        "ipc": s["ipc"],
                        **{
                            path.removeprefix("core0.pipeline.stalls."): n
                            for path, n in stalls.items()
                        },
                    }
                )
        print(Stats.tabulate(rows, title=program))
        print()


if __name__ == "__main__":
    main()
