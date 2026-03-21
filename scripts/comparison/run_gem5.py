#!/usr/bin/env python3
"""
Run gem5 on a set of binaries and save stats to results/gem5.json.

Each benchmark is run as a separate gem5 subprocess to avoid the
"multiple Root instances" limitation.

Usage:
    python scripts/comparison/run_gem5.py [binary.elf ...]

Requires gem5.opt to be on PATH (or set GEM5_BIN env var).
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent
RESULTS_DIR = Path(__file__).parent / "results"
SINGLE_SCRIPT = Path(__file__).parent / "gem5_single.py"

BENCH = ROOT / "software/bin/benchmarks"

DEFAULT_BINARIES = [
    BENCH / "mix_matrix_mul.elf",
    BENCH / "cache_linear_read.elf",
    BENCH / "cache_strided_read.elf",
    BENCH / "cache_thrash_assoc.elf",
    BENCH / "cache_write_heavy.elf",
    BENCH / "bp_random.elf",
    BENCH / "bp_pattern_alt.elf",
    BENCH / "bp_always_taken.elf",
    BENCH / "bp_never_taken.elf",
    BENCH / "alu_int_mul.elf",
    BENCH / "alu_int_div.elf",
    BENCH / "alu_fp_add.elf",
    BENCH / "pipe_load_use.elf",
    BENCH / "pipe_raw_hazard.elf",
    BENCH / "mem_rand_walk.elf",
]

GEM5_BIN = os.environ.get("GEM5_BIN", shutil.which("gem5.opt") or "gem5.opt")


def extract_stats(stats_path: Path) -> dict:
    """Parse gem5 stats.txt and extract the metrics we care about."""
    text = stats_path.read_text()
    stats = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) < 2:
            continue
        key, val = parts[0], parts[1]
        try:
            v = float(val)
        except ValueError:
            continue

        if key.endswith("ipc"):
            stats["ipc"] = v
        elif "committedInsts" in key and "ipc" not in key:
            stats["insts"] = int(v)
        elif "numCycles" in key and "core" in key:
            stats["cycles"] = int(v)
        elif "branchMispredicts" in key:
            stats["mispreds"] = int(v)
        elif "branchPredLookups" in key:
            stats["bp_lookups"] = int(v)

    if "mispreds" in stats and "bp_lookups" in stats and stats["bp_lookups"] > 0:
        stats["bp_acc"] = (1 - stats["mispreds"] / stats["bp_lookups"]) * 100

    return stats


def run_single(binary: Path, m5out: Path) -> dict:
    """Run gem5 on a single binary in a subprocess."""
    m5out.mkdir(parents=True, exist_ok=True)

    result = subprocess.run(
        [GEM5_BIN, f"--outdir={m5out}", str(SINGLE_SCRIPT), str(binary), str(m5out)],
        capture_output=True,
        text=True,
        timeout=600,
    )

    if result.returncode != 0:
        # Print stderr for debugging but don't abort
        for line in result.stderr.splitlines():
            if "fatal" in line.lower() or "error" in line.lower():
                print(f"    {line}", file=sys.stderr)

    stats_file = m5out / "stats.txt"
    if stats_file.exists():
        return extract_stats(stats_file)
    return {}


def main():
    binaries = [Path(a) for a in sys.argv[1:]] if len(sys.argv) > 1 else DEFAULT_BINARIES
    missing = [b for b in binaries if not b.exists()]
    if missing:
        for m in missing:
            print(f"error: binary not found: {m}", file=sys.stderr)
        sys.exit(1)

    print("Running gem5...")
    results = {}
    for binary in binaries:
        name = binary.stem
        m5out = Path(f"/tmp/gem5_compare_{name}")
        print(f"  gem5: {name}...", end=" ", flush=True)

        stats = run_single(binary, m5out)
        results[name] = stats

        if "ipc" in stats:
            print(f"IPC={stats['ipc']:.4f}")
        else:
            print("[no stats]")

    RESULTS_DIR.mkdir(exist_ok=True)
    out = RESULTS_DIR / "gem5.json"
    out.write_text(json.dumps(results, indent=2))
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
