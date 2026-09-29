#!/usr/bin/env python3
"""
Compare rvsim and gem5 results.

Usage:
    python tools/gem5_compare/compare.py

Reads results/rvsim.json and results/gem5.json (variant -> program -> stats)
and prints, per variant, each program's stats on both sides, then the
largest cycle gaps across all variants.
"""

import json
import sys
from pathlib import Path

RESULTS_DIR = Path(__file__).parent / "results"

# (stat, column header): shown as rvsim/gem5 and the relative difference.
COLUMNS = [
    ("cycles", "cycles"),
    ("mispredicts", "mispred"),
    ("l1i_misses", "L1I miss"),
    ("l1d_misses", "L1D miss"),
    ("l2_misses", "L2 miss"),
]


def load(name: str) -> dict:
    path = RESULTS_DIR / f"{name}.json"
    if not path.exists():
        sys.exit(f"error: {path} not found. Run run_{name}.py first.")
    return json.loads(path.read_text())


def rel(a, b):
    if a is None or b is None:
        return None
    if b == 0:
        return 0.0 if a == 0 else None
    return (a - b) / b * 100


def fmt_rel(value) -> str:
    return "n/a" if value is None else f"{value:+.0f}%"


def fmt_pair(a, b) -> str:
    def num(v):
        return "-" if v is None else f"{v:,.0f}"

    return f"{num(a)}/{num(b)}"


def print_variant(name: str, rv: dict, g5: dict) -> None:
    print(f"\n== {name} (rvsim/gem5) ==")
    header = f"{'program':<20} {'insts':>7} {'IPC':>11}"
    for _, title in COLUMNS:
        header += f" {title:>22}"
    print(header)
    for program in sorted(set(rv) | set(g5)):
        r, g = rv.get(program, {}), g5.get(program, {})
        insts_match = "=" if r.get("insts") == g.get("insts") else "!="
        ipc = f"{r.get('ipc') or 0:.2f}/{g.get('ipc') or 0:.2f}"
        line = f"{program:<20} {insts_match:>7} {ipc:>11}"
        for stat, _ in COLUMNS:
            cell = f"{fmt_pair(r.get(stat), g.get(stat))} {fmt_rel(rel(r.get(stat), g.get(stat)))}"
            line += f" {cell:>22}"
        print(line)


def print_worst(rv_all: dict, g5_all: dict, count: int = 25) -> None:
    gaps = []
    for variant, programs in rv_all.items():
        for program, r in programs.items():
            g = g5_all.get(variant, {}).get(program, {})
            gap = rel(r.get("cycles"), g.get("cycles"))
            if gap is not None:
                gaps.append((abs(gap), gap, variant, program))
    gaps.sort(reverse=True)
    print("\n== largest cycle gaps (rvsim vs gem5) ==")
    for _, gap, variant, program in gaps[:count]:
        print(f"  {gap:+7.1f}%  {variant:<22} {program}")
    if gaps:
        mean = sum(g[0] for g in gaps) / len(gaps)
        print(f"\n  mean |cycle gap| over {len(gaps)} runs: {mean:.1f}%")


def main():
    rv_all = load("rvsim")
    g5_all = load("gem5")
    for variant in rv_all:
        print_variant(variant, rv_all[variant], g5_all.get(variant, {}))
    print_worst(rv_all, g5_all)
    print("\n'=' in insts: both committed the same instruction count.")


if __name__ == "__main__":
    main()
