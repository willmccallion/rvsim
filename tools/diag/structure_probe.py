#!/usr/bin/env python3
"""Measure a core's out-of-order structures the way hardware reviews do: a
configuration calibrated to measured hardware (`calibration.py`) or a
preset.

Henry Wong's test: two independent chases of cold misses, with a number of
filler instructions of one kind after each load. While the first chase's
load, the fillers and the second chase's load fit in the structure the
fillers use up, the two misses overlap; past that, an iteration takes two
memory latencies instead of one. The probe searches for the filler count
where that happens and reports the structure's capacity: that count plus
the entries the loop's own instructions take between the two loads. Runs
bare-metal; needs riscv64-elf-gcc on PATH (the repo's nix shell).

Usage:
    python tools/diag/structure_probe.py --preset cortex_a72
    python tools/diag/structure_probe.py --preset p550 --kinds rob,load-queue
"""

import argparse
import random
import struct
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Config, presets
from tools.diag.calibration import CALIBRATED
from tools.diag.probe_build import OUT, Paging, build, cycles

SRC = Path(__file__).parent / "structure_probe" / "window.c"
CHAINS = OUT / "window_chains.bin"
PRESETS = {**CALIBRATED, **presets.PRESETS}
ITERS = (50, 100)
LINE_BYTES = 64
# The chains spread over this much memory, far past any modelled cache.
REGION_BYTES = 16 << 20
MAX_FILLERS = 512


class Kind(NamedTuple):
    """A filler kind: its code in the kernel, and the entries the loop's own
    instructions take in the same structure from the first load to the
    second, the second included."""

    code: int
    loop_entries: int


KINDS = {
    "rob": Kind(1, 3),
    "int-regs": Kind(2, 3),
    "fp-regs": Kind(3, 0),
    "load-queue": Kind(4, 2),
    "store-queue": Kind(5, 0),
    "scheduler": Kind(6, 2),
}


def write_chains(nodes_per_chain: int) -> None:
    """Two chains of `nodes_per_chain` cold lines each, interleaved in
    random order over the region, behind a header of their first nodes."""
    lines = REGION_BYTES // LINE_BYTES
    slots = random.Random(1).sample(range(1, lines), 2 * nodes_per_chain)
    words = [0] * (REGION_BYTES // 8)
    for chain in (0, 1):
        offsets = [slot * LINE_BYTES for slot in slots[chain::2]]
        words[chain] = offsets[0]
        for here, there in zip(offsets, offsets[1:] + offsets[-1:], strict=True):
            words[here // 8] = there
    OUT.mkdir(parents=True, exist_ok=True)
    CHAINS.write_bytes(struct.pack(f"<{len(words)}Q", *words))


def cycles_per_iteration(config: Config, kind: Kind, fillers: int) -> float:
    """Cycles an iteration takes with `fillers` fillers after each load."""
    short, long = (
        cycles(
            build(
                SRC,
                {"KIND": str(kind.code), "FILLERS": str(fillers), "ITERS": str(n)},
                Paging.BARE,
            ),
            config,
            Paging.BARE,
        )
        for n in ITERS
    )
    return (long - short) / (ITERS[1] - ITERS[0])


def last_overlapping(config: Config, kind: Kind) -> int | None:
    """The most fillers with which the two misses still overlap, by
    bisection on the step from one memory latency to two; None if they
    still overlap at MAX_FILLERS."""
    overlapped = cycles_per_iteration(config, kind, 0)
    if cycles_per_iteration(config, kind, MAX_FILLERS) < 1.5 * overlapped:
        return None
    low, high = 0, MAX_FILLERS
    while high - low > 1:
        middle = (low + high) // 2
        if cycles_per_iteration(config, kind, middle) < 1.5 * overlapped:
            low = middle
        else:
            high = middle
    return low


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--kinds", default=",".join(KINDS))
    args = ap.parse_args()

    config = PRESETS[args.preset]()
    write_chains(ITERS[1] + 1)
    print(f"{'structure':>12}{'fillers':>9}{'capacity':>10}", flush=True)
    for name in args.kinds.split(","):
        kind = KINDS[name]
        fillers = last_overlapping(config, kind)
        if fillers is None:
            print(f"{name:>12}{'>' + str(MAX_FILLERS):>9}{'':>10}", flush=True)
            continue
        print(f"{name:>12}{fillers:>9}{fillers + kind.loop_entries:>10}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
