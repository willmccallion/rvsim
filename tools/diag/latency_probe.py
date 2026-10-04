#!/usr/bin/env python3
"""Measure a preset's load-to-use latency at each cache level.

Builds a dependent pointer chase over footprints that fit the L1D, L2, L3
and memory, runs each twice with different step counts, and reports the
cycles per dependent load as the difference, which cancels the
initialisation. Needs riscv64-elf-gcc on PATH (the repo's nix shell).

Usage:
    python tools/diag/latency_probe.py --preset p550
    python tools/diag/latency_probe.py --preset cortex_a72 --footprints 16K,512K,8M,32M
"""

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from rvsim import Environment, presets

SRC = Path(__file__).parent / "latency_probe" / "chase.c"
OUT = ROOT / "tests" / "builds" / "latency-probe"
LIBC = ROOT / "software" / "libc"
CFLAGS = [
    "-march=rv64gc",
    "-mabi=lp64d",
    "-mcmodel=medany",
    "-ffreestanding",
    "-nostdlib",
    "-O2",
]
PRESETS = {"p550": presets.p550, "cortex_a72": presets.cortex_a72, "fast": presets.fast}
STEPS = (10_000, 20_000)


def parse_size(text: str) -> int:
    units = {"K": 1 << 10, "M": 1 << 20, "G": 1 << 30}
    return int(text[:-1]) * units[text[-1]] if text[-1] in units else int(text)


def build(footprint: int, steps: int) -> Path:
    OUT.mkdir(parents=True, exist_ok=True)
    stem = OUT / f"chase_{footprint}_{steps}"
    crt0 = OUT / "crt0.o"
    if not crt0.exists():
        subprocess.run(
            ["riscv64-elf-gcc", *CFLAGS, "-c", str(LIBC / "crt0.s"), "-o", str(crt0)],
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
            str(crt0),
            f"{stem}.o",
            "-o",
            f"{stem}.elf",
        ],
        check=True,
    )
    return Path(f"{stem}.elf")


def cycles(binary: Path, config) -> int:
    result = Environment(binary=str(binary), config=config).run(quiet=True)
    return int(result.stats.get("cycles"))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preset", choices=sorted(PRESETS), default="p550")
    ap.add_argument("--footprints", default="8K,128K,2M,16M")
    args = ap.parse_args()

    config = PRESETS[args.preset]()
    # The chase's array can run past the default stack; put the stack at the
    # top of the presets' 256 MB of RAM.
    config.initial_sp = 0x8000_0000 + (256 << 20) - 0x1000
    print(f"{'footprint':>10} {'cycles/load':>12}", flush=True)
    for text in args.footprints.split(","):
        footprint = parse_size(text)
        short, long = (cycles(build(footprint, steps), config) for steps in STEPS)
        per_load = (long - short) / (STEPS[1] - STEPS[0])
        print(f"{text:>10} {per_load:>12.1f}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
