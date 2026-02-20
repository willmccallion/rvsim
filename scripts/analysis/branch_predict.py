#!/usr/bin/env python3
"""Compare branch predictor accuracy and IPC impact at a fixed pipeline width.

Usage:
    .venv/bin/python scripts/analysis/branch_predict.py
    .venv/bin/python scripts/analysis/branch_predict.py --width 4
    .venv/bin/python scripts/analysis/branch_predict.py --programs maze qsort --width 2
"""

import argparse

from rvsim import BranchPredictor, Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]

PREDICTORS = {
    "Static": BranchPredictor.Static,
    "GShare": BranchPredictor.GShare,
    "TAGE": BranchPredictor.TAGE,
    "Perceptron": BranchPredictor.Perceptron,
    "Tournament": BranchPredictor.Tournament,
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--width", type=int, default=1, help="Pipeline width (default: 1)")
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    for program in args.programs:
        binary = f"software/bin/programs/{program}.elf"
        rows = {}
        for bp_name, bp_cls in PREDICTORS.items():
            print(f"  {program} {bp_name}...", flush=True)
            config = Config(branch_predictor=bp_cls(), uart_quiet=True, width=args.width)
            result = Environment(binary=binary, config=config).run(quiet=True, limit=args.limit)
            s = result.stats
            rows[bp_name] = Stats({
                "cycles": s["cycles"],
                "ipc": s["ipc"],
                "bp_acc%": s["branch_accuracy_pct"],
                "mispred": s["branch_mispredictions"],
                "correct": s["branch_predictions"],
            })
        print(Stats.tabulate(rows, title=f"{program} — w{args.width}"))
        print()


if __name__ == "__main__":
    main()
