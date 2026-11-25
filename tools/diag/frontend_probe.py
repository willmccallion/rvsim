#!/usr/bin/env python3
"""Measure a core's branch target buffers and return stack the way hardware
reviews do: a configuration calibrated to measured hardware
(`calibration.py`) or a preset.

`btb` runs a number of taken jumps in a row at several spacings and reports
the cycles per jump: each step up in the curve is a level of the branch
target buffers running out, or the instruction cache. `ras` runs a call
chain of a number of frames that then returns, and reports the cycles per
call and return pair and the returns that mispredict, which start once the
chain outgrows the return stack.
Each runs twice with different repeat counts, and the difference cancels
the set-up. Runs bare-metal; needs riscv64-elf-gcc on PATH (the repo's nix
shell).

Usage:
    python tools/diag/frontend_probe.py --preset cortex_a72
    python tools/diag/frontend_probe.py --preset p550 --tests ras
"""

import argparse
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Config, presets
from tools.diag.calibration import CALIBRATED
from tools.diag.probe_build import Paging, build, run

KERNELS = Path(__file__).parent / "branch_probe"
PRESETS = {**CALIBRATED, **presets.PRESETS}
BRANCH_COUNTS = (8, 16, 24, 32, 48, 64, 96, 128, 256, 512, 1024, 2048, 4096, 6144, 8192)
SPACINGS = (8, 16, 64)
DEPTHS = (4, 8, 12, 14, 15, 16, 17, 18, 24, 28, 30, 31, 32, 33, 34, 40, 48)
# Events each run measures beyond its short twin: enough to dwarf noise.
EVENTS_PER_RUN = 16384
MISPREDICTS = "core0.bp.committed.mispredicts"


class PerEvent(NamedTuple):
    """What each event of a kernel costs."""

    cycles: float
    mispredicts: float


def per_event(
    kernel: str, defines: dict[str, str], events: int, config: Config
) -> PerEvent:
    """The cost of each event of `kernel`, whose one repeat makes `events`."""
    repeats = max(2, EVENTS_PER_RUN // events)
    short, long = (
        run(
            build(KERNELS / kernel, {**defines, "ITERS": str(n)}, Paging.BARE),
            config,
            Paging.BARE,
        )
        for n in (repeats, 2 * repeats)
    )
    total = repeats * events
    return PerEvent(
        (long.cycles - short.cycles) / total,
        (long[MISPREDICTS] - short[MISPREDICTS]) / total,
    )


def print_btb(config: Config) -> None:
    """Cycles per taken jump for each branch count and spacing."""
    print(
        f"{'branches':>9}" + "".join(f"{str(s) + ' B':>9}" for s in SPACINGS),
        flush=True,
    )
    for count in BRANCH_COUNTS:
        row = ""
        for spacing in SPACINGS:
            defines = {"BRANCHES": str(count), "SPACING": str(spacing)}
            row += f"{per_event('btb.c', defines, count, config).cycles:>9.2f}"
        print(f"{count:>9}{row}", flush=True)


def print_ras(config: Config) -> None:
    """Cycles per call and return pair, and how many of the returns
    mispredict, for each call depth."""
    print(f"{'depth':>9}{'cycles':>9}{'mispredicted':>14}", flush=True)
    for depth in DEPTHS:
        cost = per_event("ras.c", {"DEPTH": str(depth)}, depth, config)
        print(
            f"{depth:>9}{cost.cycles:>9.2f}{cost.mispredicts * depth:>14.1f}",
            flush=True,
        )


TESTS = {"btb": print_btb, "ras": print_ras}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--tests", default=",".join(TESTS))
    args = ap.parse_args()

    config = PRESETS[args.preset]()
    for name in args.tests.split(","):
        print(f"== {name}", flush=True)
        TESTS[name](config)
    return 0


if __name__ == "__main__":
    sys.exit(main())
