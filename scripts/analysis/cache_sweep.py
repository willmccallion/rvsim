#!/usr/bin/env python3
"""Sweep L1 D-cache size and measure miss rate / IPC impact.

Usage:
    .venv/bin/python scripts/analysis/cache_sweep.py
    .venv/bin/python scripts/analysis/cache_sweep.py --sizes 1KB 2KB 4KB 8KB 16KB 32KB
    .venv/bin/python scripts/analysis/cache_sweep.py --programs qsort --ways 1 2 4
"""

import argparse

from rvsim import Cache, Config, Environment, Stats

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
SIZES = ["1KB", "2KB", "4KB", "8KB", "16KB", "32KB"]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sizes", nargs="+", default=SIZES, help="D-cache sizes to sweep")
    ap.add_argument("--ways", type=int, default=4, help="Cache associativity (default: 4)")
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    for program in args.programs:
        binary = f"software/bin/programs/{program}.elf"
        rows = {}
        for size in args.sizes:
            print(f"  {program} dcache={size}...", flush=True)
            config = Config(
                uart_quiet=True,
                l1d=Cache(size=size, ways=args.ways),
            )
            result = Environment(binary=binary, config=config).run(quiet=True, limit=args.limit)
            s = result.stats
            total_dc = s["dcache_hits"] + s["dcache_misses"]
            miss_rate = s["dcache_misses"] / total_dc * 100 if total_dc > 0 else 0
            rows[size] = Stats({
                "cycles": s["cycles"],
                "ipc": s["ipc"],
                "dc_hits": s["dcache_hits"],
                "dc_misses": s["dcache_misses"],
                "dc_miss%": miss_rate,
            })
        print(Stats.tabulate(rows, title=f"{program} — D-cache sweep ({args.ways}-way)"))
        print()


if __name__ == "__main__":
    main()
