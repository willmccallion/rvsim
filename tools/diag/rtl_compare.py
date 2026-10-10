#!/usr/bin/env python3
"""
Compare rvsim's retire timing with an RTL core's on the same ELFs.

Usage:
    python tools/diag/rtl_compare.py --core rocket|boom [--config PRESET]
        [--programs NAME ...] [--threshold CYCLES] [--jobs N] [--out FILE]

Runs each program in tests/builds/rtl-programs (tools/rtl/programs/build.sh)
on the core's Chipyard Verilator simulator (make rtl-build) with its retire
trace on, and on rvsim with the core's preset and the commit log open. The
two traces are lined up instruction by instruction from the ELF's entry to
the exit spin, and each program reports the cycle ratio, where the
per-instruction retire-cycle difference first passes the threshold, and
the PCs where most of the difference accumulates.
"""

import argparse
import bisect
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor, ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT))

PROGRAMS = ROOT / "tests/builds/rtl-programs"
SIMULATORS = ROOT / "tests/builds/chipyard/sims/verilator"
RESULTS = ROOT / "tests/builds/results"
NM = os.environ.get("RISCV_NM", "riscv64-elf-nm")

#: `jal x0, 0`: the spin every workload ends in after its tohost write.
SPIN = 0x0000006F


@dataclass(frozen=True)
class Retirement:
    cycle: int
    pc: int
    inst: int


@dataclass(frozen=True)
class Core:
    """An RTL core: its Chipyard config, rvsim preset, trace line format and
    whether its trace keeps an instruction that trapped (rvsim's log does)."""

    config: str
    preset: str
    line: re.Pattern
    logs_trapped: bool

    @property
    def simulator(self) -> Path:
        return SIMULATORS / f"simulator-chipyard.harness-{self.config}"


# Rocket under +verbose (rocket/RocketCore.scala, `csr.io.trace(0).valid`):
# `C0: <cycle> [1] pc=[<pc>] W[...] R[...] R[...] inst=[<inst>] DASM(...)`.
ROCKET_LINE = re.compile(
    r"^C0:\s+(?P<cycle>\d+) \[1\] pc=\[(?P<pc>[0-9a-f]+)\].* inst=\[(?P<inst>[0-9a-f]+)\]"
)
# BOOM with enableCommitLogPrintf and tools/rtl/patches/boom-commit-log-cycle
# (v4/exu/core.scala): `<cycle> <priv> 0x<pc> (0x<inst>) [x<n> 0x<value>]`.
BOOM_LINE = re.compile(
    r"^\s*(?P<cycle>\d+) \d 0x(?P<pc>[0-9a-f]+) \(0x(?P<inst>[0-9a-f]+)\)"
)
# rvsim's commit log (uarch/pipeline/commit_log.rs).
RVSIM_LINE = re.compile(
    r"^core   0: 0x(?P<pc>[0-9a-f]+) \(0x(?P<inst>[0-9a-f]+)\) priv \d cycle (?P<cycle>\d+)"
)

CORES = {
    "rocket": Core(
        config="RocketConfig", preset="rocket", line=ROCKET_LINE, logs_trapped=False
    ),
    "boom": Core(
        config="RvsimMediumBoomV4Config",
        preset="boom",
        line=BOOM_LINE,
        logs_trapped=True,
    ),
}


def parse_trace(text: str, line: re.Pattern) -> list[Retirement]:
    retirements = []
    for raw in text.splitlines():
        match = line.match(raw)
        if match:
            retirements.append(
                Retirement(
                    int(match["cycle"]), int(match["pc"], 16), int(match["inst"], 16)
                )
            )
    return retirements


def parse_rvsim_log(text: str, keep_trapped: bool) -> list[Retirement]:
    """rvsim's commit log as retirements. rvsim logs a trapping instruction's
    line before its trap line; `keep_trapped` says whether the core's trace
    has that instruction too."""
    retirements = []
    faulted = False
    for raw in text.splitlines():
        if raw.startswith("core   0: trap "):
            if faulted and not keep_trapped:
                retirements.pop()
            faulted = False
            continue
        match = RVSIM_LINE.match(raw)
        faulted = match is not None
        if match:
            retirements.append(
                Retirement(
                    int(match["cycle"]), int(match["pc"], 16), int(match["inst"], 16)
                )
            )
    return retirements


def elf_entry(elf: Path) -> int:
    header = elf.read_bytes()[:32]
    if header[:4] != b"\x7fELF" or header[4] != 2:
        raise ValueError(f"{elf} is not a 64-bit ELF")
    return struct.unpack_from("<Q", header, 24)[0]


def program_window(retirements: list[Retirement], entry: int) -> list[Retirement]:
    """The retirements from the ELF's entry up to its exit spin."""
    start = next((i for i, r in enumerate(retirements) if r.pc == entry), None)
    if start is None:
        raise ValueError(f"the entry {entry:#x} never retired")
    end = next(
        (i for i in range(start, len(retirements)) if retirements[i].inst == SPIN),
        len(retirements),
    )
    return retirements[start:end]


def run_rtl(core: Core, elf: Path, max_cycles: int) -> list[Retirement]:
    with tempfile.NamedTemporaryFile(suffix=".trace") as trace:
        # +loadmem puts the ELF straight into the simulated DRAM; without it
        # the harness feeds it through the serial link, a cycle per few bytes.
        result = subprocess.run(
            [
                str(core.simulator),
                "+permissive",
                "+verbose",
                f"+max-cycles={max_cycles}",
                f"+loadmem={elf}",
                "+permissive-off",
                str(elf),
            ],
            stdout=subprocess.PIPE,
            stderr=trace,
            text=True,
            check=False,
        )
        # The harness reports on stderr, after the trace.
        text = Path(trace.name).read_text()
        if "*** PASSED ***" not in text:
            raise RuntimeError(
                f"{core.config} did not pass {elf.name}: "
                f"{(result.stdout + text).strip()[-300:]}"
            )
        return parse_trace(text, core.line)


def run_rvsim(
    preset: str, elf: Path, max_cycles: int, keep_trapped: bool
) -> list[Retirement]:
    from rvsim import presets
    from rvsim._core import Simulator
    from rvsim.config._config import _config_to_dict

    config = getattr(presets, preset, None)
    if config is None:
        raise RuntimeError(f"rvsim has no preset {preset!r}")
    with tempfile.NamedTemporaryFile(suffix=".log") as log:
        cpu = Simulator(_config_to_dict(config()), elf_data=elf.read_bytes())
        cpu.open_commit_log(log.name)
        exit_code = cpu.run(limit=max_cycles, stats_sections=None)
        del cpu
        if exit_code is None:
            raise RuntimeError(f"rvsim ran {elf.name} past {max_cycles} cycles")
        return parse_rvsim_log(Path(log.name).read_text(), keep_trapped)


class Symbols:
    """The ELF's text symbols, for naming a PC."""

    def __init__(self, elf: Path):
        self.addresses: list[int] = []
        self.names: list[str] = []
        try:
            output = subprocess.run(
                [NM, "-n", str(elf)], capture_output=True, text=True, check=True
            ).stdout
        except (OSError, subprocess.CalledProcessError):
            return
        for line in output.splitlines():
            parts = line.split()
            if len(parts) == 3 and parts[1] in "tT":
                self.addresses.append(int(parts[0], 16))
                self.names.append(parts[2])

    def name(self, pc: int) -> str:
        index = bisect.bisect_right(self.addresses, pc) - 1
        if index < 0:
            return f"{pc:#x}"
        return f"{self.names[index]}+{pc - self.addresses[index]:#x}"


def uncompressed(inst: int) -> bool:
    return inst & 3 == 3


@dataclass
class Comparison:
    program: str
    insts: int
    rtl_cycles: int
    rvsim_cycles: int
    divergence: dict | None
    first_growth: dict | None
    contributors: list[dict]

    @property
    def ratio(self) -> float:
        return self.rvsim_cycles / self.rtl_cycles if self.rtl_cycles else float("nan")

    def to_dict(self) -> dict:
        return {
            "insts": self.insts,
            "rtl_cycles": self.rtl_cycles,
            "rvsim_cycles": self.rvsim_cycles,
            "ratio": self.ratio,
            "divergence": self.divergence,
            "first_growth": self.first_growth,
            "contributors": self.contributors,
        }


def compare(
    program: str,
    rtl: list[Retirement],
    rvsim: list[Retirement],
    symbols: Symbols,
    threshold: int,
) -> Comparison:
    """Lines the two windows up and measures where their timing parts."""
    divergence = None
    aligned = min(len(rtl), len(rvsim))
    for i in range(aligned):
        same_pc = rtl[i].pc == rvsim[i].pc
        same_inst = rtl[i].inst == rvsim[i].inst or not (
            uncompressed(rtl[i].inst) and uncompressed(rvsim[i].inst)
        )
        if not (same_pc and same_inst):
            divergence = {
                "index": i,
                "rtl": f"{rtl[i].pc:#x} ({rtl[i].inst:#x})",
                "rvsim": f"{rvsim[i].pc:#x} ({rvsim[i].inst:#x})",
            }
            aligned = i
            break
    if divergence is None and len(rtl) != len(rvsim):
        divergence = {
            "index": aligned,
            "rtl": f"{len(rtl)} retirements",
            "rvsim": f"{len(rvsim)} retirements",
        }
    if aligned == 0:
        return Comparison(program, 0, 0, 0, divergence, None, [])

    rtl_start, rvsim_start = rtl[0].cycle, rvsim[0].cycle
    deltas = [
        (rvsim[i].cycle - rvsim_start) - (rtl[i].cycle - rtl_start)
        for i in range(aligned)
    ]
    first_growth = None
    for i, delta in enumerate(deltas):
        if abs(delta) >= threshold:
            first_growth = {
                "index": i,
                "pc": f"{rtl[i].pc:#x}",
                "symbol": symbols.name(rtl[i].pc),
                "delta": delta,
                "rtl_cycle": rtl[i].cycle - rtl_start,
                "rvsim_cycle": rvsim[i].cycle - rvsim_start,
            }
            break

    growth_by_pc: dict[int, int] = defaultdict(int)
    count_by_pc: dict[int, int] = defaultdict(int)
    for i in range(1, aligned):
        growth_by_pc[rtl[i].pc] += deltas[i] - deltas[i - 1]
        count_by_pc[rtl[i].pc] += 1
    contributors = [
        {
            "pc": f"{pc:#x}",
            "symbol": symbols.name(pc),
            "retirements": count_by_pc[pc],
            "delta": growth,
        }
        for pc, growth in sorted(growth_by_pc.items(), key=lambda item: -abs(item[1]))[
            :5
        ]
        if growth != 0
    ]
    last = aligned - 1
    return Comparison(
        program,
        aligned,
        rtl[last].cycle - rtl_start,
        rvsim[last].cycle - rvsim_start,
        divergence,
        first_growth,
        contributors,
    )


def print_table(comparisons: list[Comparison], threshold: int) -> None:
    print(
        f"{'program':<20} {'insts':>8} {'rtl cycles':>11} {'rvsim cycles':>13}"
        f" {'ratio':>6} {'rtl IPC':>8} {'rvsim IPC':>10}  first |delta| >= {threshold}"
    )
    for c in comparisons:
        if c.insts == 0:
            print(
                f"{c.program:<20} diverged before the first instruction: {c.divergence}"
            )
            continue
        growth = "-"
        if c.first_growth:
            growth = (
                f"#{c.first_growth['index']} {c.first_growth['symbol']}"
                f" ({c.first_growth['delta']:+d})"
            )
        print(
            f"{c.program:<20} {c.insts:>8,} {c.rtl_cycles:>11,} {c.rvsim_cycles:>13,}"
            f" {c.ratio:>6.3f} {c.insts / c.rtl_cycles:>8.3f}"
            f" {c.insts / c.rvsim_cycles:>10.3f}  {growth}"
        )
        if c.divergence:
            print(
                f"{'':<20} diverged at #{c.divergence['index']}: "
                f"rtl {c.divergence['rtl']}, rvsim {c.divergence['rvsim']}"
            )
        for contributor in c.contributors[:3]:
            print(
                f"{'':<20} {contributor['delta']:>+9,} cycles over"
                f" {contributor['retirements']:,} retirements at"
                f" {contributor['symbol']} ({contributor['pc']})"
            )
    ratios = [c.ratio for c in comparisons if c.insts]
    if ratios:
        mean_error = sum(abs(r - 1) for r in ratios) / len(ratios)
        print(f"\nmean |ratio - 1| over {len(ratios)} programs: {mean_error:.1%}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[1])
    parser.add_argument("--core", choices=CORES, required=True)
    parser.add_argument("--config", help="rvsim preset (default: the core's)")
    parser.add_argument("--programs", nargs="*", help="program names (default: all)")
    parser.add_argument("--threshold", type=int, default=50)
    parser.add_argument("--max-cycles", type=int, default=50_000_000)
    parser.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 4))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()

    core = CORES[args.core]
    preset = args.config or core.preset
    if not core.simulator.exists():
        sys.exit(f"error: {core.simulator} not built; run make rtl-build")
    elfs = sorted(PROGRAMS.glob("*.elf"))
    if args.programs:
        elfs = [elf for elf in elfs if elf.stem in args.programs]
    if not elfs:
        sys.exit(f"error: no programs in {PROGRAMS}; run tools/rtl/programs/build.sh")

    with (
        ThreadPoolExecutor(args.jobs) as rtl_pool,
        ProcessPoolExecutor(args.jobs) as rvsim_pool,
    ):
        rtl_runs = {
            elf: rtl_pool.submit(run_rtl, core, elf, args.max_cycles) for elf in elfs
        }
        rvsim_runs = {
            elf: rvsim_pool.submit(
                run_rvsim, preset, elf, args.max_cycles, core.logs_trapped
            )
            for elf in elfs
        }
        comparisons = []
        failures = {}
        for elf in elfs:
            try:
                entry = elf_entry(elf)
                rtl = program_window(rtl_runs[elf].result(), entry)
                rvsim = program_window(rvsim_runs[elf].result(), entry)
            except (RuntimeError, ValueError, OSError) as error:
                failures[elf.stem] = str(error)
                print(f"  {args.core} {elf.stem:22} failed: {error}", flush=True)
                continue
            comparison = compare(elf.stem, rtl, rvsim, Symbols(elf), args.threshold)
            comparisons.append(comparison)
            print(
                f"  {args.core} {elf.stem:22} ratio {comparison.ratio:.3f}", flush=True
            )

    print()
    print_table(comparisons, args.threshold)
    for program, error in failures.items():
        print(f"{program:<20} failed: {error}")
    out = args.out or RESULTS / f"rtl-{args.core}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(
            {
                "core": args.core,
                "config": core.config,
                "preset": preset,
                "threshold": args.threshold,
                "programs": {c.program: c.to_dict() for c in comparisons},
                "failures": failures,
            },
            indent=2,
        )
    )
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
