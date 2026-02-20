#!/usr/bin/env python3
"""Measure IPC scaling across pipeline widths for a fixed branch predictor.

Usage:
    .venv/bin/python scripts/analysis/width_scaling.py
    .venv/bin/python scripts/analysis/width_scaling.py --bp TAGE --widths 1 2 4 8
    .venv/bin/python scripts/analysis/width_scaling.py --programs mandelbrot qsort
"""

import argparse

from rvsim import BranchPredictor, Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
WIDTHS = [1, 2, 4]

BP_MAP = {
    "Static": BranchPredictor.Static,
    "GShare": BranchPredictor.GShare,
    "TAGE": BranchPredictor.TAGE,
    "Perceptron": BranchPredictor.Perceptron,
    "Tournament": BranchPredictor.Tournament,
}


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--bp",
        default="TAGE",
        choices=BP_MAP.keys(),
        help="Branch predictor (default: TAGE)",
    )
    ap.add_argument(
        "--widths", type=int, nargs="+", default=WIDTHS, help="Pipeline widths to test"
    )
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    bp = BP_MAP[args.bp]()

    for program in args.programs:
        binary = f"software/bin/programs/{program}.elf"
        rows = {}
        for width in args.widths:
            label = f"w{width}"
            print(f"  {program} {label}...", flush=True)
            config = Config(branch_predictor=bp, uart_quiet=True, width=width)
            result = Environment(binary=binary, config=config).run(
                quiet=True, limit=args.limit
            )
            s = result.stats
            rows[label] = Stats(
                {
                    "cycles": s["cycles"],
                    "ipc": s["ipc"],
                    "bp_acc%": s["branch_accuracy_pct"],
                    "mispred": s["branch_mispredictions"],
                }
            )
        print(Stats.tabulate(rows, title=f"{program} — {args.bp}"))
        print()


if __name__ == "__main__":
    main()
