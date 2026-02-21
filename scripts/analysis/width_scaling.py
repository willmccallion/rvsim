#!/usr/bin/env python3
"""Measure IPC scaling across pipeline widths for a fixed branch predictor.

Usage:
    .venv/bin/python scripts/analysis/width_scaling.py
    .venv/bin/python scripts/analysis/width_scaling.py --bp TAGE --widths 1 2 4 8
    .venv/bin/python scripts/analysis/width_scaling.py --programs mandelbrot qsort
"""

import argparse
import time

from rvsim import BranchPredictor, Config, Sweep

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
    binaries = [f"software/bin/programs/{p}.elf" for p in args.programs]
    configs = {
        f"w{w}": Config(branch_predictor=bp, uart_quiet=True, width=w)
        for w in args.widths
    }

    n_jobs = len(binaries) * len(configs)
    print(f"Width scaling: {len(binaries)} binaries x {len(configs)} widths "
          f"({args.bp}, {n_jobs} runs, limit={args.limit:,})")

    t0 = time.perf_counter()
    results = Sweep(binaries=binaries, configs=configs).run(
        parallel=True, limit=args.limit
    )
    elapsed = time.perf_counter() - t0
    print(f"Completed in {elapsed:.1f}s\n")

    results.compare(
        metrics=["cycles", "ipc", "branch_accuracy_pct", "branch_mispredictions"],
        baseline=f"w{args.widths[0]}",
        col_header="width",
    )


if __name__ == "__main__":
    main()
