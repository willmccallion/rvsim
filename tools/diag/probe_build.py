"""Builds and runs the hardware probes' C kernels on rvsim: bare-metal, with
no address translation, or as a static Linux program in SE mode under 4 KB
pages. Each kernel is compiled per parameter set and timed by the difference
between two runs of different lengths, which cancels its set-up."""

import enum
import subprocess
from pathlib import Path

from rvsim import Config, Simulator
from rvsim._core import StatsTree

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "tests" / "builds" / "hw-probes"
LIBC = ROOT / "software" / "libc"
LIBC_SOURCES = ["crt0_htif.s", "stdio.c", "stdlib.c", "htif.c", "fdt.c"]
# Bare kernels can outgrow the linker script's stack; it goes at the top of
# the calibrated configurations' 256 MB of RAM, below the device tree.
STACK_TOP = 0x8000_0000 + (256 << 20) - (64 << 10) - 0x1000
ARCH = ["-march=rv64gc", "-mabi=lp64d", "-O2"]


class Paging(enum.Enum):
    """How a kernel's addresses are translated."""

    BARE = "bare"
    PAGES_4K = "4K pages"


def build(source: Path, defines: dict[str, str], paging: Paging) -> Path:
    """`source` compiled with `defines` for `paging`'s run."""
    OUT.mkdir(parents=True, exist_ok=True)
    tag = "_".join(f"{key}{value}" for key, value in sorted(defines.items()))
    flags = [f"-D{key}={value}" for key, value in defines.items()]
    if paging is Paging.PAGES_4K:
        elf = OUT / f"{source.stem}_se_{tag}.elf"
        compile_cmd = ["riscv64-linux-gnu-gcc", *ARCH, "-static", *flags, str(source)]
        subprocess.run([*compile_cmd, "-o", str(elf)], check=True)
        return elf
    return build_bare(source, flags, OUT / f"{source.stem}_{tag}")


def build_bare(source: Path, flags: list[str], stem: Path) -> Path:
    """`source` linked against the bare-metal libc, as `stem`.elf."""
    bare = [*ARCH, "-mcmodel=medany", "-ffreestanding", "-nostdlib"]
    libc = [OUT / f"{name}.o" for name in LIBC_SOURCES]
    for name, obj in zip(LIBC_SOURCES, libc, strict=True):
        if not obj.exists():
            subprocess.run(
                ["riscv64-elf-gcc", *bare, "-c", str(LIBC / name), "-o", str(obj)],
                check=True,
            )
    obj = stem.with_suffix(".o")
    subprocess.run(
        ["riscv64-elf-gcc", *bare, *flags, "-c", str(source), "-o", str(obj)],
        check=True,
    )
    subprocess.run(
        [
            "riscv64-elf-ld",
            "--no-warn-rwx-segments",
            "-T",
            str(LIBC / "user.ld"),
            f"--defsym=_stack_top={STACK_TOP:#x}",
            *map(str, libc),
            str(obj),
            "-o",
            f"{stem}.elf",
        ],
        check=True,
    )
    return Path(f"{stem}.elf")


def run(binary: Path, config: Config, paging: Paging) -> StatsTree:
    """The statistics of running `binary` to its exit on `config`."""
    mode = "se" if paging is Paging.PAGES_4K else "bare"
    sim = Simulator(config, binary=str(binary), mode=mode)
    exit_code = sim.run()
    if exit_code != 0:
        raise RuntimeError(f"{binary.name} exited with {exit_code}")
    return sim.stats


def cycles(binary: Path, config: Config, paging: Paging) -> int:
    """The cycles `binary` takes on `config`."""
    return run(binary, config, paging).cycles


def parse_size(text: str) -> int:
    """`text` as bytes: a count, or one with a K, M or G suffix."""
    units = {"K": 1 << 10, "M": 1 << 20, "G": 1 << 30}
    return int(text[:-1]) * units[text[-1]] if text[-1] in units else int(text)
