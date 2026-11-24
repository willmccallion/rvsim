#!/usr/bin/env python3
"""Checks rvsim's accuracy against the stored references.

Reruns rvsim's side of the gem5 comparison (every variant, against
tools/gem5_compare/results/gem5.json) and of the Rocket and BOOM RTL
comparisons (against tools/accuracy/rtl_references.json, the RTL's cycles
from tools/diag/rtl_compare.py), then fails if any program's absolute error
has grown past its budget in tools/accuracy/budget.json.

Usage:
    python tools/accuracy/check.py [--out FILE]
    python tools/accuracy/check.py --write-budget

`--write-budget` sets each budget to the program's error now plus one
percentage point, for a change that improves accuracy to lock its gain in.
"""

import argparse
import json
import os
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import TypedDict

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tools/gem5_compare"))

import results
import run_rvsim
from variants import VARIANTS

from tools.diag import rtl_compare

HERE = Path(__file__).resolve().parent
BUDGET = HERE / "budget.json"
RTL_REFERENCES = HERE / "rtl_references.json"
# Points of absolute error a program may gain on its error now before the
# check fails: room for cycle noise, not for a regression.
BUDGET_SLACK = 1.0
RTL_CYCLE_LIMIT = 50_000_000

# Suite ("gem5 base", "rtl boom") -> program -> signed error in percent.
Errors = dict[str, dict[str, float]]


class RtlProgram(TypedDict):
    insts: int
    rtl_cycles: int


class RtlCore(TypedDict):
    config: str
    preset: str
    programs: dict[str, RtlProgram]


def error_percent(measured: float, reference: float) -> float:
    return (measured - reference) / reference * 100


def gem5_errors() -> Errors:
    """rvsim's error against gem5 for every variant and program."""
    reference = results.load("gem5")
    jobs = [(v, b) for v in VARIANTS for b in results.program_binaries()]
    errors: Errors = {f"gem5 {v}": {} for v in VARIANTS}
    with ProcessPoolExecutor() as pool:
        for variant, program, stats in pool.map(run_rvsim.run_one, jobs):
            gem5_cycles = reference[variant][program]["cycles"]
            cycles = stats["cycles"]
            if gem5_cycles is None or cycles is None:
                raise RuntimeError(f"{program} on {variant} has no cycle count")
            errors[f"gem5 {variant}"][program] = error_percent(cycles, gem5_cycles)
    return errors


def rvsim_rtl_error(job: tuple[str, str, RtlProgram, bool]) -> tuple[str, float]:
    """rvsim's error against the RTL's cycles for one program, over the
    retirements from its entry to its exit spin, as rtl_compare.py counts."""
    preset, program, reference, keep_trapped = job
    elf = rtl_compare.PROGRAMS / f"{program}.elf"
    retirements = rtl_compare.run_rvsim(preset, elf, RTL_CYCLE_LIMIT, keep_trapped)
    window = rtl_compare.program_window(retirements, rtl_compare.elf_entry(elf))
    if len(window) != reference["insts"]:
        raise RuntimeError(
            f"{program} on {preset} retired {len(window)} instructions,"
            f" the RTL reference {reference['insts']}; rebuild tools/rtl/programs"
        )
    cycles = window[-1].cycle - window[0].cycle
    return program, error_percent(cycles, reference["rtl_cycles"])


def rtl_errors() -> Errors:
    """rvsim's error against each RTL core's stored cycles."""
    references: dict[str, RtlCore] = json.loads(RTL_REFERENCES.read_text())
    errors: Errors = {}
    for core, stored in references.items():
        keep_trapped = rtl_compare.CORES[core].logs_trapped
        jobs = [
            (stored["preset"], name, ref, keep_trapped)
            for name, ref in stored["programs"].items()
        ]
        with ProcessPoolExecutor() as pool:
            errors[f"rtl {core}"] = dict(pool.map(rvsim_rtl_error, jobs))
    return errors


def over_budget(errors: Errors, budget: Errors) -> list[str]:
    """Each program whose absolute error exceeds its budget, or has none."""
    failures = []
    for suite, programs in errors.items():
        for program, error in programs.items():
            allowed = budget.get(suite, {}).get(program)
            if allowed is None:
                failures.append(f"{suite} {program}: {error:+.1f}% has no budget")
            elif abs(error) > allowed:
                failures.append(
                    f"{suite} {program}: {error:+.1f}%, budget {allowed:.1f}%"
                )
    return failures


def budget_from(errors: Errors) -> Errors:
    return {
        suite: {
            program: round(abs(error) + BUDGET_SLACK, 1)
            for program, error in programs.items()
        }
        for suite, programs in errors.items()
    }


def report(errors: Errors, budget: Errors) -> str:
    """Each suite's table of error against budget, as markdown."""
    lines = []
    for suite, programs in errors.items():
        mean = sum(abs(e) for e in programs.values()) / len(programs)
        lines += [
            f"### {suite}: mean absolute error {mean:.1f}%",
            "",
            "| program | error | budget |",
            "|---|---:|---:|",
        ]
        for program, error in sorted(programs.items()):
            allowed = budget.get(suite, {}).get(program)
            shown = "none" if allowed is None else f"{allowed:.1f}%"
            lines.append(f"| `{program}` | {error:+.1f}% | {shown} |")
        lines.append("")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, help="write the errors as JSON here")
    ap.add_argument(
        "--write-budget", action="store_true", help="set the budgets from this run"
    )
    args = ap.parse_args()

    errors = {**gem5_errors(), **rtl_errors()}
    if args.out:
        args.out.write_text(json.dumps(errors, indent=2, sort_keys=True))
    if args.write_budget:
        BUDGET.write_text(
            json.dumps(budget_from(errors), indent=2, sort_keys=True) + "\n"
        )
        print(f"wrote {BUDGET}")
        return 0

    budget: Errors = json.loads(BUDGET.read_text())
    table = report(errors, budget)
    print(table)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write("## Accuracy\n\n" + table)
    failures = over_budget(errors, budget)
    for failure in failures:
        print(f"OVER BUDGET {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
