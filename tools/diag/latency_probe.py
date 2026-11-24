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
import enum
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Config, Simulator, presets
from tools.diag.calibration import CALIBRATED

SRC = Path(__file__).parent / "latency_probe" / "chase.c"
OUT = ROOT / "tests" / "builds" / "latency-probe"
LIBC = ROOT / "software" / "libc"
LIBC_SOURCES = ["crt0_htif.s", "stdio.c", "stdlib.c", "htif.c", "fdt.c"]
STACK_TOP = 0x8000_0000 + (256 << 20) - (64 << 10) - 0x1000
CFLAGS = [
    "-march=rv64gc",
    "-mabi=lp64d",
    "-mcmodel=medany",
    "-ffreestanding",
    "-nostdlib",
    "-O2",
]
PRESETS = {**CALIBRATED, **presets.PRESETS}
STEPS = (10_000, 20_000)


class Paging(enum.Enum):
    """How the chase's addresses are translated."""

    BARE = "bare"
    PAGES_4K = "4K pages"


def parse_size(text: str) -> int:
    units = {"K": 1 << 10, "M": 1 << 20, "G": 1 << 30}
    return int(text[:-1]) * units[text[-1]] if text[-1] in units else int(text)


def build(footprint: int, steps: int, paging: Paging) -> Path:
    OUT.mkdir(parents=True, exist_ok=True)
    if paging is Paging.PAGES_4K:
        return build_se(footprint, steps)
    stem = OUT / f"chase_{footprint}_{steps}"
    libc = [OUT / f"{name}.o" for name in LIBC_SOURCES]
    for source, obj in zip(LIBC_SOURCES, libc, strict=True):
        if not obj.exists():
            subprocess.run(
                ["riscv64-elf-gcc", *CFLAGS, "-c", str(LIBC / source), "-o", str(obj)],
                check=True,
            )
    subprocess.run(
        [
            "riscv64-elf-gcc",
            *CFLAGS,
            f"-DFOOTPRINT={footprint}UL",
            f"-DSTEPS={steps}UL",
            "-c",
            str(SRC),
            "-o",
            f"{stem}.o",
        ],
        check=True,
    )
    subprocess.run(
        [
            "riscv64-elf-ld",
            "--no-warn-rwx-segments",
            "-T",
            str(LIBC / "user.ld"),
            # The chase's array can run past the linker script's stack; put
            # the stack at the top of the presets' 256 MB of RAM, below the
            # device tree.
            f"--defsym=_stack_top={STACK_TOP:#x}",
            *map(str, libc),
            f"{stem}.o",
            "-o",
            f"{stem}.elf",
        ],
        check=True,
    )
    return Path(f"{stem}.elf")


def build_se(footprint: int, steps: int) -> Path:
    """The chase as a static Linux binary, for SE mode."""
    elf = OUT / f"chase_se_{footprint}_{steps}.elf"
    subprocess.run(
        [
            "riscv64-linux-gnu-gcc",
            "-march=rv64gc",
            "-mabi=lp64d",
            "-O2",
            "-static",
            f"-DFOOTPRINT={footprint}UL",
            f"-DSTEPS={steps}UL",
            str(SRC),
            "-o",
            str(elf),
        ],
        check=True,
    )
    return elf


def cycles(binary: Path, config: Config, paging: Paging) -> int:
    mode = "se" if paging is Paging.PAGES_4K else "bare"
    sim = Simulator(config, binary=str(binary), mode=mode)
    exit_code = sim.run()
    if exit_code != 0:
        raise RuntimeError(f"{binary.name} exited with {exit_code}")
    return sim.stats.cycles


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


def cycles_per_load(footprint: int, config: Config, paging: Paging) -> float:
    """Cycles per dependent load over `footprint` bytes."""
    short, long = (
        cycles(build(footprint, steps, paging), config, paging) for steps in STEPS
    )
    return (long - short) / (STEPS[1] - STEPS[0])


if __name__ == "__main__":
    sys.exit(main())
