#!/usr/bin/env python3
"""Sweep L1 D-cache size and measure miss rate / IPC impact.

Usage:
    .venv/bin/python scripts/analysis/cache_sweep.py
    .venv/bin/python scripts/analysis/cache_sweep.py --sizes 1KB 2KB 4KB 8KB 16KB 32KB
    .venv/bin/python scripts/analysis/cache_sweep.py --programs qsort --ways 1 2 4
"""

import argparse
import time

from rvsim import Cache, Config, Sweep

PROGRAMS = ["mandelbrot", "maze", "qsort", "merge_sort"]
SIZES = ["1KB", "2KB", "4KB", "8KB", "16KB", "32KB"]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sizes", nargs="+", default=SIZES, help="D-cache sizes to sweep")
    ap.add_argument("--ways", type=int, default=4, help="Cache associativity (default: 4)")
    ap.add_argument("--programs", nargs="+", default=PROGRAMS, help="Programs to run")
    ap.add_argument("--limit", type=int, default=50_000_000, help="Cycle limit")
    args = ap.parse_args()

    binaries = [f"software/bin/programs/{p}.elf" for p in args.programs]
    configs = {
        size: Config(uart_quiet=True, l1d=Cache(size=size, ways=args.ways, mshr_count=8))
        for size in args.sizes
    }

    n_jobs = len(binaries) * len(configs)
    print(f"D-cache sweep: {len(binaries)} binaries x {len(configs)} sizes "
          f"({args.ways}-way, {n_jobs} runs, limit={args.limit:,})")

    t0 = time.perf_counter()
    results = Sweep(binaries=binaries, configs=configs).run(
        parallel=True, limit=args.limit
    )
    elapsed = time.perf_counter() - t0
    print(f"Completed in {elapsed:.1f}s\n")

    results.compare(
        metrics=["cycles", "ipc", "dcache_hits", "dcache_misses"],
        baseline=args.sizes[0],
        col_header="L1D size",
    )


if __name__ == "__main__":
    main()
