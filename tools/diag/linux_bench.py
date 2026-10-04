#!/usr/bin/env python3
"""Run the rootfs's userspace benchmarks on a core preset, from the cached
boot, and print each one's score, derived from the cycles it took, next to
its cycles, instructions and IPC.

Usage:
    python tools/diag/linux_bench.py                    # p550, one hart
    python tools/diag/linux_bench.py --preset cortex_a72 --harts 2
    python tools/diag/linux_bench.py --only dhrystone,coremark --json out.json

The first run on a preset boots to the login shell (minutes); the boot is
cached and later runs start from it.
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Session, presets

# Each benchmark's command, how many of its units the command runs, and
# how its score follows from that count and the cycles the run took. The
# scores come from the simulator's cycle count, not the guest's clock,
# whose resolution is too coarse for runs this short; both are the per-MHz
# figures hardware is quoted in (DMIPS/MHz divides by 1757 Dhrystones).
BENCHMARKS = {
    "dhrystone": (
        "dhrystone 300000",
        300_000,
        "DMIPS/MHz",
        lambda runs, mcycles: runs / mcycles / 1757.0,
    ),
    "coremark": (
        "coremark 0x0 0x0 0x66 200 7 1 2000",
        200,
        "CoreMark/MHz",
        lambda iters, mcycles: iters / mcycles,
    ),
}

PRESETS = {
    "p550": presets.p550,
    "cortex_a72": presets.cortex_a72,
    "fast": presets.fast,
}


def score_of(name: str, cycles: int) -> float:
    _, count, _, formula = BENCHMARKS[name]
    return formula(count, cycles / 1e6)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--harts", type=int, default=1)
    ap.add_argument("--only", default=None, help="comma-separated benchmark names")
    ap.add_argument("--json", default=None, help="write the results here")
    ap.add_argument("--echo", action="store_true", help="show the guest console")
    args = ap.parse_args()

    names = args.only.split(",") if args.only else list(BENCHMARKS)
    unknown = [n for n in names if n not in BENCHMARKS]
    if unknown:
        ap.error(f"unknown benchmark(s): {', '.join(unknown)}")

    core = PRESETS[args.preset]()
    config = presets.linux(harts=args.harts, core=core)
    session = Session.linux(config, echo=args.echo, progress=True)
    boot = session.fast_forward(until=Session.LOGIN_SHELL).last_fast_forward
    print(
        f"booted on {args.preset} x{args.harts}: {boot.cycle:,} cycles"
        f" ({'cached' if boot.cached else f'{boot.host_seconds:.0f} s'})",
        file=sys.stderr,
    )

    rows = []
    print(
        f"{'benchmark':10} {'score':>14} {'unit':12} {'cycles':>13} {'insts':>13} {'ipc':>6}"
    )
    for name in names:
        command, _, unit, _ = BENCHMARKS[name]
        region = session.measure(command, name=name)
        score = score_of(name, region.cycles)
        row = {
            "benchmark": name,
            "command": command,
            "score": score,
            "unit": unit,
            "cycles": region.cycles,
            "instructions": region.instructions,
            "ipc": region.ipc,
            "mhz": config.cpu_clock_mhz,
        }
        rows.append(row)
        shown = f"{score:,.2f}"
        print(
            f"{name:10} {shown:>14} {unit:12} {region.cycles:>13,}"
            f" {region.instructions:>13,} {region.ipc:>6.2f}"
        )
    session.close()

    if args.json:
        Path(args.json).write_text(
            json.dumps(
                {"preset": args.preset, "harts": args.harts, "results": rows}, indent=1
            )
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
