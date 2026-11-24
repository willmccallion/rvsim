#!/usr/bin/env python3
"""Measure a core's load-to-use latency at each cache level: a configuration
calibrated to measured hardware (`calibration.py`) or a preset.

Builds a dependent pointer chase over footprints that fit the L1D, L2, L3
and memory, runs each twice with different step counts, and reports the
cycles per dependent load as the difference, which cancels the
initialisation. Each footprint runs twice over: bare-metal, with no address
translation, as a measurement with huge pages sees memory; and in SE mode
under 4 KB pages, where TLB misses and page walks add their cost. Needs
riscv64-elf-gcc and riscv64-linux-gnu-gcc on PATH (the repo's nix shell).

Usage:
    python tools/diag/latency_probe.py --preset p550
    python tools/diag/latency_probe.py --preset cortex_a72 --footprints 16K,512K,8M,32M
"""

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Config, presets
from tools.diag.calibration import CALIBRATED
from tools.diag.probe_build import Paging, build, cycles, parse_size

SRC = Path(__file__).parent / "latency_probe" / "chase.c"
PRESETS = {**CALIBRATED, **presets.PRESETS}
STEPS = (10_000, 20_000)


def cycles_per_load(footprint: int, config: Config, paging: Paging) -> float:
    """Cycles per dependent load over `footprint` bytes."""
    runs = [
        build(SRC, {"FOOTPRINT": f"{footprint}UL", "STEPS": f"{steps}UL"}, paging)
        for steps in STEPS
    ]
    short, long = (cycles(binary, config, paging) for binary in runs)
    return (long - short) / (STEPS[1] - STEPS[0])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--footprints", default="8K,128K,2M,16M")
    args = ap.parse_args()

    config = PRESETS[args.preset]()
    print(f"{'footprint':>10}" + "".join(f"{p.value:>12}" for p in Paging), flush=True)
    for text in args.footprints.split(","):
        footprint = parse_size(text)
        row = "".join(
            f"{cycles_per_load(footprint, config, paging):>12.1f}" for paging in Paging
        )
        print(f"{text:>10}{row}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
