#!/usr/bin/env python3
"""Show pipeline stall breakdown (memory, control, data) across configurations.

Usage:
    .venv/bin/python scripts/analysis/stall_breakdown.py
    .venv/bin/python scripts/analysis/stall_breakdown.py --widths 1 2 4
"""

import argparse

from rvsim import BranchPredictor, Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
BP_MAP = {
    "Static": BranchPredictor.Static,
    "TAGE": BranchPredictor.TAGE,
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--widths", type=int, nargs="+", default=[1, 2, 4], help="Pipeline widths")
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
                result = Environment(binary=binary, config=config).run(quiet=True, limit=args.limit)
                s = result.stats
                rows[label] = Stats({
                    "cycles": s["cycles"],
                    "ipc": s["ipc"],
                    "stalls_mem": s["stalls_mem"],
                    "stalls_ctrl": s["stalls_control"],
                    "stalls_data": s["stalls_data"],
                })
        print(Stats.tabulate(rows, title=program))
        print()


if __name__ == "__main__":
    main()
