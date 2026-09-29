#!/usr/bin/env python3
"""
Run gem5 on the comparison programs under every variant and save the stats
to results/gem5.json.

Each run is a separate gem5 process, since gem5 allows one Root per process.

Usage:
    python tools/gem5_compare/run_gem5.py [variant ...]

Runs every variant in variants.VARIANTS when none are named. Requires
gem5.opt on PATH, or GEM5_BIN set.
"""

import json
import os
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from variants import VARIANTS

ROOT = Path(__file__).parent.parent.parent
RESULTS_DIR = Path(__file__).parent / "results"
SINGLE_SCRIPT = Path(__file__).parent / "gem5_single.py"
PROGRAMS = ROOT / "tests/builds/compare-programs"
OUTDIR = ROOT / "tests/builds/results/gem5-compare"

GEM5_BIN = os.environ.get("GEM5_BIN", shutil.which("gem5.opt") or "gem5.opt")

CORE = "board.processor.cores.core"
CACHES = "board.cache_hierarchy"
STATS = {
    "insts": f"{CORE}.commitStats0.numInsts",
    "cycles": f"{CORE}.numCycles",
    "ipc": f"{CORE}.ipc",
    "branches": f"{CORE}.commitStats0.committedControl::IsControl",
    # Committed branches and jumps whose prediction was wrong, as rvsim counts.
    "mispredicts": f"{CORE}.branchPred.mispredicted_0::total",
    "loads": f"{CORE}.commitStats0.numLoadInsts",
    "stores": f"{CORE}.commitStats0.numStoreInsts",
    "l1i_misses": f"{CACHES}.l1i-cache-0.demandMisses::total",
    "l1d_accesses": f"{CACHES}.l1d-cache-0.demandAccesses::total",
    "l1d_misses": f"{CACHES}.l1d-cache-0.demandMisses::total",
    "l2_accesses": f"{CACHES}.l2-cache-0.demandAccesses::total",
    "l2_misses": f"{CACHES}.l2-cache-0.demandMisses::total",
}


def extract_stats(stats_path: Path) -> dict:
    values = {}
    for line in stats_path.read_text().splitlines():
        parts = line.split()
        if len(parts) >= 2:
            values[parts[0]] = parts[1]
    return {
        name: float(values[key]) if key in values else None
        for name, key in STATS.items()
    }


def run_one(job: tuple[str, Path]) -> tuple[str, str, dict]:
    variant_name, binary = job
    m5out = OUTDIR / variant_name / binary.stem
    m5out.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [
            GEM5_BIN,
            f"--outdir={m5out}",
            str(SINGLE_SCRIPT),
            str(binary),
            str(m5out),
            variant_name,
        ],
        capture_output=True,
        text=True,
        timeout=1800,
    )
    (m5out / "run.log").write_text(result.stdout + result.stderr)
    stats_file = m5out / "stats.txt"
    if result.returncode != 0 or not stats_file.exists():
        return variant_name, binary.stem, {}
    return variant_name, binary.stem, extract_stats(stats_file)


def main():
    variants = sys.argv[1:] or list(VARIANTS)
    binaries = sorted(PROGRAMS.glob("*.elf"))
    if not binaries:
        sys.exit(
            f"error: no programs in {PROGRAMS}; run tools/gem5_compare/programs/build.sh"
        )

    jobs = [(v, b) for v in variants for b in binaries]
    results: dict = {v: {} for v in variants}
    with ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
        for variant_name, program, stats in pool.map(run_one, jobs):
            results[variant_name][program] = stats
            ipc = (
                f"IPC={stats['ipc']:.3f}" if stats.get("ipc") is not None else "FAILED"
            )
            print(f"  gem5 {variant_name:20} {program:22} {ipc}", flush=True)

    RESULTS_DIR.mkdir(exist_ok=True)
    out = RESULTS_DIR / "gem5.json"
    # Variants not run this time keep their earlier results.
    saved = json.loads(out.read_text()) if out.exists() else {}
    saved.update(results)
    out.write_text(json.dumps(saved, indent=2, sort_keys=True))
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
