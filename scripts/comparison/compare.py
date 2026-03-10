#!/usr/bin/env python3
"""
Compare rvsim and gem5 results.

Usage:
    python scripts/comparison/compare.py

Reads results/rvsim.json and results/gem5.json and prints a table.
"""

import json
import sys
from pathlib import Path

RESULTS_DIR = Path(__file__).parent / "results"


def load(name: str) -> dict:
    path = RESULTS_DIR / f"{name}.json"
    if not path.exists():
        print(f"error: {path} not found. Run run_{name}.py first.", file=sys.stderr)
        sys.exit(1)
    return json.loads(path.read_text())


def pct_diff(a, b):
    if a is None or b is None or b == 0:
        return "n/a"
    return f"{(a - b) / b * 100:+.1f}%"


def fmt(val, fmt_str):
    return format(val, fmt_str) if val is not None else "n/a"


def main():
    rv = load("rvsim")
    g5 = load("gem5")

    all_names = sorted(set(rv) | set(g5))

    col = 14
    print(f"\n{'Binary':<{col}} {'rvsim IPC':>10} {'gem5 IPC':>10} {'IPC diff':>9}  {'rvsim BP%':>10} {'gem5 BP%':>9}  {'rvsim cycles':>14} {'gem5 cycles':>13} {'cycle diff':>10}")
    print("-" * 115)

    for name in all_names:
        r = rv.get(name, {})
        g = g5.get(name, {})

        rv_ipc  = r.get("ipc")
        g5_ipc  = g.get("ipc")
        rv_cyc  = r.get("cycles")
        g5_cyc  = g.get("cycles")
        rv_bp   = r.get("bp_acc")
        g5_bp   = g.get("bp_acc")

        print(
            f"{name:<{col}}"
            f" {fmt(rv_ipc, '10.4f')}"
            f" {fmt(g5_ipc, '10.4f')}"
            f" {pct_diff(rv_ipc, g5_ipc):>9}"
            f"  {fmt(rv_bp, '9.2f')}%"
            f" {fmt(g5_bp, '8.2f')}%"
            f"  {fmt(rv_cyc, '14,')}"
            f" {fmt(g5_cyc, '13,')}"
            f" {pct_diff(rv_cyc, g5_cyc):>10}"
        )

    print()
    print("IPC diff = (rvsim - gem5) / gem5. Negative means rvsim predicts fewer IPC than gem5.")


if __name__ == "__main__":
    main()
