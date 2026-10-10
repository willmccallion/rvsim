#!/usr/bin/env python3
"""
Measure what each one-mechanism kernel isolates, on an RTL core or rvsim.

Usage:
    python tools/diag/rtl_probe.py --core rocket|boom [--config PRESET]
        [--probes NAME ...]

Runs each probe in tests/builds/rtl-programs with its retire trace on and
reports the cycles between consecutive retirements of the instruction the
probe chains (a dependent load, multiply, divide or FP add), or from the
instruction it exercises (a CSR write, FENCE, FENCE.I, ECALL, branch) to
the next retirement, as the modes of that gap. The numbers are what the
rvsim presets cite for a latency the RTL source does not state.
"""

import argparse
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from rtl_compare import (
    CORES,
    PROGRAMS,
    Retirement,
    elf_entry,
    program_window,
    run_rtl,
    run_rvsim,
)


def opcode(inst: int) -> int:
    return inst & 0x7F


def funct3(inst: int) -> int:
    return (inst >> 12) & 7


def funct7(inst: int) -> int:
    return inst >> 25


def rd(inst: int) -> int:
    return (inst >> 7) & 0x1F


def rs1(inst: int) -> int:
    return (inst >> 15) & 0x1F


def is_self_dependent_ld(inst: int) -> bool:
    """`ld rd, (rs1)` with rd == rs1, as `ld` or as the raw `c.ld` the cores
    log (rvsim logs the expanded instruction)."""
    if inst & 0xE003 == 0x6000:
        return (inst >> 2) & 7 == (inst >> 7) & 7
    return opcode(inst) == 0x03 and funct3(inst) == 3 and rd(inst) == rs1(inst)


def is_mul(inst: int) -> bool:
    return opcode(inst) == 0x33 and funct7(inst) == 1 and funct3(inst) == 0


def is_div(inst: int) -> bool:
    return opcode(inst) == 0x33 and funct7(inst) == 1 and funct3(inst) in (4, 5)


def is_fadd_d(inst: int) -> bool:
    return opcode(inst) == 0x53 and funct7(inst) == 0x01


def is_fdiv_d(inst: int) -> bool:
    return opcode(inst) == 0x53 and funct7(inst) == 0x0D


def is_csrw_mscratch(inst: int) -> bool:
    return opcode(inst) == 0x73 and funct3(inst) == 1 and (inst >> 20) == 0x340


def is_csrr_mscratch(inst: int) -> bool:
    return opcode(inst) == 0x73 and funct3(inst) == 2 and (inst >> 20) == 0x340


def is_fence(inst: int) -> bool:
    return opcode(inst) == 0x0F and funct3(inst) == 0


def is_fence_i(inst: int) -> bool:
    return opcode(inst) == 0x0F and funct3(inst) == 1


def is_mret(inst: int) -> bool:
    return inst == 0x30200073


def is_branch(inst: int) -> bool:
    return opcode(inst) == 0x63


def is_handler_entry(inst: int) -> bool:
    """`csrr t0, mepc`, the first instruction of trap_return's handler."""
    return inst == 0x341022F3


@dataclass(frozen=True)
class Probe:
    """A kernel, the instruction it exercises, and which gap to measure:
    `chain` between consecutive such instructions, `after` from one to
    whatever retires next, `before` from whatever retired last to it."""

    program: str
    what: str
    matches: callable
    gap: str


PROBES = [
    Probe("chase_l1", "L1D load-to-use", is_self_dependent_ld, "chain"),
    Probe("chase_l2", "L2 load-to-use", is_self_dependent_ld, "chain"),
    Probe("chase_mem", "memory load-to-use", is_self_dependent_ld, "chain"),
    Probe("mul_chain", "mul latency", is_mul, "chain"),
    Probe("div_chain", "divu latency plus loop (48-bit operands)", is_div, "chain"),
    Probe("fp_add_chain", "fadd.d latency plus loop", is_fadd_d, "chain"),
    Probe("fp_div_chain", "fdiv.d latency plus loop", is_fdiv_d, "chain"),
    Probe(
        "csr_serialize", "csrw mscratch to next retirement", is_csrw_mscratch, "after"
    ),
    Probe(
        "csr_serialize", "csrr mscratch to next retirement", is_csrr_mscratch, "after"
    ),
    Probe("fence", "fence to next retirement", is_fence, "after"),
    Probe("fence_i", "fence.i to next retirement", is_fence_i, "after"),
    Probe(
        "trap_return", "last retirement before the handler", is_handler_entry, "before"
    ),
    Probe("trap_return", "mret to next retirement", is_mret, "after"),
    Probe("br_random", "branch to next retirement", is_branch, "after"),
]


def gaps(window: list[Retirement], probe: Probe) -> Counter:
    counts: Counter = Counter()
    previous = None
    for i, r in enumerate(window):
        if not probe.matches(r.inst):
            continue
        if probe.gap == "chain":
            if previous is not None:
                counts[r.cycle - previous.cycle] += 1
            previous = r
        elif probe.gap == "after" and i + 1 < len(window):
            counts[window[i + 1].cycle - r.cycle] += 1
        elif probe.gap == "before" and i > 0:
            counts[r.cycle - window[i - 1].cycle] += 1
    return counts


def describe(counts: Counter) -> str:
    total = sum(counts.values())
    if not total:
        return "no matching instruction retired"
    modes = counts.most_common(3)
    text = ", ".join(f"{gap} cycles x{n} ({n / total:.0%})" for gap, n in modes)
    return f"{total} gaps: {text}"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[1])
    parser.add_argument("--core", choices=CORES, required=True)
    parser.add_argument("--config", help="measure rvsim's preset instead of the RTL")
    parser.add_argument("--probes", nargs="*", help="probe names (default: all)")
    parser.add_argument("--max-cycles", type=int, default=50_000_000)
    args = parser.parse_args()

    core = CORES[args.core]
    probes = [p for p in PROBES if not args.probes or p.program in args.probes]
    side = args.config or core.config
    for probe in probes:
        elf = PROGRAMS / f"{probe.program}.elf"
        if not elf.exists():
            print(f"{probe.program:<16} missing; run tools/rtl/programs/build.sh")
            continue
        if args.config:
            retirements = run_rvsim(
                args.config, elf, args.max_cycles, core.logs_trapped
            )
        else:
            retirements = run_rtl(core, elf, args.max_cycles)
        window = program_window(retirements, elf_entry(elf))
        print(
            f"{probe.program:<16} {side}: {probe.what}: {describe(gaps(window, probe))}",
            flush=True,
        )


if __name__ == "__main__":
    main()
