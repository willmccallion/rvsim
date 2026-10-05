#!/usr/bin/env python3
"""Record or compare cycle counts for a fixed program x config matrix.

Every (program, config) pair runs in its own subprocess through the installed
``rvsim`` module. The result file holds the cycle count, retired instruction
count and exit code of each pair, which is what "cycle-identical" means for a
single-core configuration.

A config is a label from the conformance pipelines, or ``preset <name>`` for
one of ``rvsim.presets`` (``linux`` included), so a timing change shows its
effect on every preset at once.

Usage:
    python tools/diag/cycle_baseline.py --out baseline.json
    python tools/diag/cycle_baseline.py --compare baseline.json [--out new.json]
    python tools/diag/cycle_baseline.py --programs qsort,maze --configs "o3 w4"
"""

import argparse
import concurrent.futures as cf
import json
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

PROGRAMS = {
    "qsort": "software/bin/programs/qsort.elf",
    "maze": "software/bin/programs/maze.elf",
    "mandelbrot": "software/bin/programs/mandelbrot.elf",
    "merge_sort": "software/bin/programs/merge_sort.elf",
    "fib": "software/bin/programs/fib.elf",
    "sort": "software/bin/programs/sort.elf",
    "life": "software/bin/programs/life.elf",
    "mix_matrix_mul": "software/bin/benchmarks/mix_matrix_mul.elf",
    "cache_write_heavy": "software/bin/benchmarks/cache_write_heavy.elf",
    "cache_linear_read": "software/bin/benchmarks/cache_linear_read.elf",
    "cache_thrash_assoc": "software/bin/benchmarks/cache_thrash_assoc.elf",
    "mem_rand_walk": "software/bin/benchmarks/mem_rand_walk.elf",
    "atomics": "software/bin/benchmarks/atomics.elf",
    "bp_random": "software/bin/benchmarks/bp_random.elf",
    "alu_int_div": "software/bin/benchmarks/alu_int_div.elf",
    "pipe_load_use": "software/bin/benchmarks/pipe_load_use.elf",
    "aes_bench": "software/bin/benchmarks/aes_bench.elf",
    "lz77_bench": "software/bin/benchmarks/lz77_bench.elf",
}

CONFIGS = [
    "inorder w1",
    "inorder w4",
    "o3 w4",
    "o3 w4 small-rob",
    "o3 w4 mshr-1",
    "o3 w4 no-l2",
    "o3 w4 l3",
    "o3 w4 tiny-l1",
    "o3 w4 dram",
    "o3 w4 ddr5",
    "inorder w4 ddr5",
    "preset basic",
    "preset fast",
    "preset cortex_a72",
    "preset m1",
    "preset p550",
    "preset linux",
]


def config_for(label):
    """The config `label` names: ``preset <name>`` or a pipeline label."""
    from rvsim import presets
    from tests.conformance.configs.pipelines import PIPELINES

    if label.startswith("preset "):
        name = label.removeprefix("preset ")
        if name == "linux":
            return presets.linux()
        return presets.PRESETS[name]()
    return next(c for lbl, c in PIPELINES if lbl == label)


def run_one(args):
    name, elf_path, label, limit, hart_count = args
    from rvsim._core import Simulator
    from rvsim.config._config import _config_to_dict

    cfg = config_for(label)
    cfg.uart_quiet = True
    cfg.hart_count = hart_count
    with open(os.path.join(ROOT, elf_path), "rb") as f:
        elf_data = f.read()
    t0 = time.time()
    cpu = Simulator(_config_to_dict(cfg), elf_data=elf_data)
    exit_code = cpu.run(limit=limit, stats_sections=None)
    stats = cpu.stats
    return {
        "program": name,
        "config": label,
        "cycles": stats.cycles,
        "instructions": stats.instructions_retired,
        "exit_code": exit_code,
        "seconds": round(time.time() - t0, 2),
    }


def key(r):
    return f"{r['program']} | {r['config']}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None)
    ap.add_argument("--compare", default=None)
    ap.add_argument("--programs", default=None, help="comma-separated subset")
    ap.add_argument("--configs", default=None, help="comma-separated subset of labels")
    ap.add_argument("--limit", type=int, default=30_000_000)
    ap.add_argument("--hart-count", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    args = ap.parse_args()

    programs = PROGRAMS
    if args.programs:
        wanted = args.programs.split(",")
        programs = {k: v for k, v in PROGRAMS.items() if k in wanted}
    configs = CONFIGS
    if args.configs:
        configs = [c.strip() for c in args.configs.split(",")]

    work = [
        (name, path, label, args.limit, args.hart_count)
        for name, path in programs.items()
        for label in configs
    ]
    results = []
    with cf.ProcessPoolExecutor(max_workers=args.jobs) as pool:
        for r in pool.map(run_one, work):
            results.append(r)
            print(
                f"{key(r):48s} cycles={r['cycles']:>12,d} insts={r['instructions']:>12,d} "
                f"exit={r['exit_code']} ({r['seconds']}s)",
                flush=True,
            )

    if args.out:
        with open(args.out, "w") as f:
            json.dump(results, f, indent=1)
        print(f"wrote {args.out}")

    if args.compare:
        with open(args.compare) as f:
            base = {key(r): r for r in json.load(f)}
        diffs = 0
        for r in results:
            b = base.get(key(r))
            if b is None:
                continue
            same = (b["cycles"], b["instructions"], b["exit_code"]) == (
                r["cycles"],
                r["instructions"],
                r["exit_code"],
            )
            if not same:
                diffs += 1
                print(
                    f"DIFF {key(r):48s} cycles {b['cycles']:,d} -> {r['cycles']:,d} "
                    f"({r['cycles'] - b['cycles']:+,d}) insts {b['instructions']:,d} -> "
                    f"{r['instructions']:,d} exit {b['exit_code']} -> {r['exit_code']}"
                )
        print(f"{len(results)} pairs compared, {diffs} differ")
        sys.exit(1 if diffs else 0)


if __name__ == "__main__":
    main()
