#!/usr/bin/env python3
"""Measure a core's sustained bandwidth at each cache level: a configuration
calibrated to measured hardware (`calibration.py`) or a preset.

Runs tinymembench's "C copy" and "C fill" loops and a summing read over
each footprint, twice with different pass counts, and reports the bytes a
pass moves (copied, filled or read, as tinymembench counts them) per cycle
and in MB/s at the configuration's clock. Each runs bare-metal and in SE
mode under 4 KB pages, as `latency_probe.py` does. Needs riscv64-elf-gcc and
riscv64-linux-gnu-gcc on PATH (the repo's nix shell).

Usage:
    python tools/diag/bandwidth_probe.py --preset cortex_a72
    python tools/diag/bandwidth_probe.py --preset p550 --kernels read --footprints 16K,128K
"""

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Config, presets
from tools.diag.calibration import CALIBRATED
from tools.diag.probe_build import Paging, build, cycles, parse_size

SRC = Path(__file__).parent / "bandwidth_probe" / "kernels.c"
PRESETS = {**CALIBRATED, **presets.PRESETS}
KERNELS = {"copy": 1, "fill": 2, "read": 3}
# Bytes each run moves beyond its warm-up pass: enough passes over a small
# footprint to dwarf the loop overhead, one over a large one.
BYTES_PER_RUN = 4 << 20


def bytes_per_cycle(
    kernel: str, footprint: int, config: Config, paging: Paging
) -> float:
    """Bytes a pass of `kernel` over `footprint` moves per cycle."""
    passes = max(1, BYTES_PER_RUN // footprint)
    runs = [
        build(
            SRC,
            {
                "KERNEL": str(KERNELS[kernel]),
                "FOOTPRINT": str(footprint),
                "PASSES": str(n),
            },
            paging,
        )
        for n in (passes, 2 * passes)
    ]
    short, long = (cycles(binary, config, paging) for binary in runs)
    return passes * footprint / (long - short)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--kernels", default="read,copy,fill")
    ap.add_argument("--footprints", default="16K,128K,32M")
    args = ap.parse_args()

    config = PRESETS[args.preset]()
    mhz = config.cpu_clock_mhz
    columns = "".join(f"{p.value + ' B/c':>14}{'MB/s':>9}" for p in Paging)
    print(f"{'kernel':>7}{'footprint':>11}{columns}", flush=True)
    for kernel in args.kernels.split(","):
        for text in args.footprints.split(","):
            footprint = parse_size(text)
            row = ""
            for paging in Paging:
                rate = bytes_per_cycle(kernel, footprint, config, paging)
                row += f"{rate:>14.2f}{rate * mhz:>9.0f}"
            print(f"{kernel:>7}{text:>11}{row}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
