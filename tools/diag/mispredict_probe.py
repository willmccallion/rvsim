#!/usr/bin/env python3
"""Measure the cycles a branch misprediction costs a core: a configuration
calibrated to measured hardware (`calibration.py`) or a preset.

Runs a loop whose branch follows a byte array, over random bytes and over
clear ones, each for two lengths so the difference cancels the set-up, and
divides the extra cycles the random bytes cost by the extra committed
mispredictions they cause. That is the penalty a software measurement
of hardware sees: the L1 load the branch waits for, then the redirect and
the refill, which it also reports on their own, as the out-of-order
backend times them. Runs bare-metal; needs riscv64-elf-gcc on PATH (the repo's nix
shell).

Usage:
    python tools/diag/mispredict_probe.py --preset cortex_a72
    python tools/diag/mispredict_probe.py --preset p550 --passes 8
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

SRC = Path(__file__).parent / "branch_probe" / "mispredict.c"
PRESETS = {**CALIBRATED, **presets.PRESETS}
MISPREDICTS = "core0.bp.committed.mispredicts"
RESOLVE_TO_ISSUE = "core0.bp.mispredict_penalty"
# Bytes the loop branches on, half the L1D of either calibrated core.
BYTES = 8192


def defines(random: int, passes: int) -> dict[str, str]:
    """The kernel's settings: random or clear bytes, and how many passes."""
    return {"RANDOM": str(random), "PASSES": str(passes), "N": str(BYTES)}


class Penalty(NamedTuple):
    """What a misprediction costs, as measured."""

    cycles: float
    # The part after the branch resolves, until the right path issues; only
    # the out-of-order backend times it.
    after_resolving: float | None
    # Of the random branches, the share that mispredicted.
    rate: float


def penalty(config: Config, passes: int) -> Penalty:
    """What each misprediction of the random bytes costs on `config`."""
    random, clear = (extra_passes(config, bits, passes) for bits in (1, 0))
    mispredicts = random.mispredicts - clear.mispredicts
    return Penalty(
        (random.cycles - clear.cycles) / mispredicts,
        random.after_resolving,
        mispredicts / (passes * BYTES),
    )


class Extra(NamedTuple):
    """What `passes` more passes over the bytes cost."""

    cycles: float
    mispredicts: float
    # Mean cycles from a mispredicted branch resolving to the right path issuing.
    after_resolving: float | None


def extra_passes(config: Config, random: int, passes: int) -> Extra:
    """The cost of `passes` passes beyond a first `passes`, which cancels the
    set-up and the cold start."""
    short, long = (
        run(build(SRC, defines(random, n), Paging.BARE), config, Paging.BARE)
        for n in (passes, 2 * passes)
    )
    resolving = long.histogram(RESOLVE_TO_ISSUE)
    return Extra(
        long.cycles - short.cycles,
        long[MISPREDICTS] - short[MISPREDICTS],
        resolving.mean if resolving else None,
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--passes", type=int, default=4)
    args = ap.parse_args()

    cost = penalty(PRESETS[args.preset](), args.passes)
    print(f"{cost.cycles:.1f} cycles per misprediction")
    if cost.after_resolving is not None:
        print(f"{cost.after_resolving:.1f} of them after the branch resolves")
    print(f"{cost.rate:.0%} of the random branches mispredict")
    return 0


if __name__ == "__main__":
    sys.exit(main())
