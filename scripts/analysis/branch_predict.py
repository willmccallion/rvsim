#!/usr/bin/env python3
"""Compare branch predictor accuracy and IPC impact at a fixed pipeline width.

Usage:
    .venv/bin/python scripts/analysis/branch_predict.py
    .venv/bin/python scripts/analysis/branch_predict.py --width 4
    .venv/bin/python scripts/analysis/branch_predict.py --programs maze qsort --width 2
"""

import argparse
import time

from rvsim import BranchPredictor, Config, Sweep

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]

PREDICTORS = {
    "Static": BranchPredictor.Static,
    "GShare": BranchPredictor.GShare,
    "TAGE": BranchPredictor.TAGE,
    "ScLTage": BranchPredictor.ScLTage,
    "Perceptron": BranchPredictor.Perceptron,
    "Tournament": BranchPredictor.Tournament,
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--width", type=int, default=1, help="Pipeline width (default: 1)")
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    binaries = [f"software/bin/programs/{p}.elf" for p in args.programs]
    configs = {
        name: Config(branch_predictor=bp_cls(), uart_quiet=True, width=args.width)
        for name, bp_cls in PREDICTORS.items()
    }

    n_jobs = len(binaries) * len(configs)
    print(f"Branch predictor comparison: {len(binaries)} binaries x {len(configs)} predictors "
          f"(w{args.width}, {n_jobs} runs, limit={args.limit:,})")

    t0 = time.perf_counter()
    results = Sweep(binaries=binaries, configs=configs).run(
        parallel=True, limit=args.limit
    )
    elapsed = time.perf_counter() - t0
    print(f"Completed in {elapsed:.1f}s\n")

    results.compare(
        metrics=["cycles", "ipc", "branch_accuracy_pct", "branch_mispredictions", "branch_predictions"],
        baseline="Static",
        col_header="predictor",
    )


if __name__ == "__main__":
    main()
