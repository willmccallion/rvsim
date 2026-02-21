#!/usr/bin/env python3
"""Measure IPC scaling across pipeline widths for InOrder and OutOfOrder backends.

Usage:
    .venv/bin/python scripts/analysis/o3_inorder.py
    .venv/bin/python scripts/analysis/o3_inorder.py --widths 1 2 4
    .venv/bin/python scripts/analysis/o3_inorder.py --programs mandelbrot qsort
"""

import argparse
import time

from rvsim import Backend, BranchPredictor, Config, Sweep

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
WIDTHS = [1, 2, 4, 8]


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--widths", type=int, nargs="+", default=WIDTHS, help="Pipeline widths to test"
    )
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    binaries = [f"software/bin/programs/{p}.elf" for p in args.programs]

    configs = {}
    for w in args.widths:
        configs[f"inorder_w{w}"] = Config(
            branch_predictor=BranchPredictor.TAGE(),
            uart_quiet=True,
            width=w,
            backend=Backend.InOrder(),
        )
        configs[f"o3_w{w}"] = Config(
            branch_predictor=BranchPredictor.TAGE(),
            uart_quiet=True,
            width=w,
            backend=Backend.OutOfOrder(),
        )

    n_jobs = len(binaries) * len(configs)
    print(
        f"Backend scaling: {len(binaries)} binaries x {len(configs)} configurations "
        f"({n_jobs} runs, limit={args.limit:,})"
    )

    t0 = time.perf_counter()
    results = Sweep(binaries=binaries, configs=configs).run(
        parallel=True, limit=args.limit
    )
    elapsed = time.perf_counter() - t0
    print(f"Completed in {elapsed:.1f}s\n")

    results.compare(
        metrics=[
            "ipc",
            "cycles",
            "instructions_retired",
            "stalls_data",
            "stalls_mem",
            "stalls_control",
        ],
        baseline=f"inorder_w{args.widths[0]}",
        col_header="config",
    )


if __name__ == "__main__":
    main()
