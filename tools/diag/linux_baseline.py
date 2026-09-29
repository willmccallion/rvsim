#!/usr/bin/env python3
"""Boot Linux for a fixed cycle budget and record where the machine got to.

At a fixed cycle count the retired-instruction count and the PC of every hart
identify the boot trajectory exactly, so two builds that agree here are
cycle-identical over the whole budget.

Usage:
    python tools/diag/linux_baseline.py --out base.json
    python tools/diag/linux_baseline.py --compare base.json
"""

import argparse
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from rvsim import Simulator, presets


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None)
    ap.add_argument("--compare", default=None)
    ap.add_argument("--limit", type=int, default=20_000_000)
    ap.add_argument("--hart-count", type=int, default=1)
    args = ap.parse_args()

    out_dir = os.path.join(ROOT, "software", "linux", "output")
    cfg = presets.linux().replace(uart_quiet=True, hart_count=args.hart_count)
    os.chdir(ROOT)
    sim = Simulator(
        cfg,
        kernel=os.path.join(out_dir, "Image"),
        disk=os.path.join(out_dir, "disk.img"),
    )
    exit_code = sim.run(limit=args.limit, stats_sections=None)
    stats = sim.stats
    result = {
        "limit": args.limit,
        "hart_count": args.hart_count,
        "cycles": stats.cycles,
        "instructions": stats.instructions_retired,
        "pc": sim.pc,
        "exit_code": exit_code,
    }
    print(json.dumps(result, indent=1))
    if args.out:
        with open(args.out, "w") as f:
            json.dump(result, f, indent=1)
    if args.compare:
        with open(args.compare) as f:
            base = json.load(f)
        same = all(
            base[k] == result[k] for k in ("cycles", "instructions", "pc", "exit_code")
        )
        print("IDENTICAL" if same else f"DIFF vs {args.compare}: {base}")
        sys.exit(0 if same else 1)


if __name__ == "__main__":
    main()
