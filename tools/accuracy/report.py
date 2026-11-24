#!/usr/bin/env python3
"""Regenerates the error tables in the docs from the comparison results.

docs/error.md takes rvsim's and gem5's results for the base variant
(tools/gem5_compare/results/), and docs/rtl-rocket.md and docs/rtl-boom.md
take the last full RTL comparison (tests/builds/results/rtl-<core>.json,
from tools/diag/rtl_compare.py). Each table and its summary sit between
`<!-- accuracy: NAME -->` and `<!-- /accuracy -->` in its page.

Usage:
    python tools/accuracy/report.py
"""

import json
import statistics
import sys
from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/gem5_compare"))

import results

DOCS = ROOT / "docs"
RTL_RESULTS = ROOT / "tests/builds/results"
END = "<!-- /accuracy -->"
RTL_CORES = {"rocket": "Rocket", "boom": "BOOM"}


def summary_of(errors: list[float]) -> tuple[float, float]:
    """The mean and median of the absolute errors."""
    absolute = [abs(e) for e in errors]
    return statistics.mean(absolute), statistics.median(absolute)


def gem5_section() -> list[str]:
    """The base variant's table against gem5 and its summary."""
    gem5 = results.load("gem5")["base"]
    rvsim = results.load("rvsim")["base"]
    lines = [
        "| Program | gem5 cycles | rvsim cycles | Error | gem5 IPC | rvsim IPC | gem5 mispredicts | rvsim mispredicts |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    errors = []
    for program in sorted(gem5):
        g, r = gem5[program], rvsim[program]
        g_cycles, r_cycles = g["cycles"] or 0.0, r["cycles"] or 0.0
        error = (r_cycles - g_cycles) / g_cycles * 100
        errors.append(error)
        lines.append(
            f"| `{program}` | {g_cycles:,.0f} | {r_cycles:,.0f} | {error:+.1f}% "
            f"| {g['ipc'] or 0:.2f} | {r['ipc'] or 0:.2f} "
            f"| {g['mispredicts'] or 0:,.0f} | {r['mispredicts'] or 0:,.0f} |"
        )
    mean, median = summary_of(errors)
    return [
        *lines,
        "",
        f"Across these {len(errors)} programs:",
        "",
        f"- **Mean absolute error:** {mean:.1f}%.",
        f"- **Median absolute error:** {median:.1f}%.",
    ]


def rtl_section(core: str) -> list[str]:
    """`core`'s table from its last full RTL comparison and its summary."""
    programs = json.loads((RTL_RESULTS / f"rtl-{core}.json").read_text())["programs"]
    name = RTL_CORES[core]
    lines = [
        f"| program | instructions | {name} cycles | rvsim cycles | error | {name} IPC | rvsim IPC | where the difference accumulates |",
        "|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    errors = []
    for program, p in sorted(programs.items()):
        error = (p["ratio"] - 1) * 100
        errors.append(error)
        where = f"`{p['contributors'][0]['symbol']}`" if p["contributors"] else ""
        lines.append(
            f"| `{program}` | {p['insts']:,} | {p['rtl_cycles']:,} | {p['rvsim_cycles']:,} "
            f"| {error:+.1f}% | {p['insts'] / p['rtl_cycles']:.2f} "
            f"| {p['insts'] / p['rvsim_cycles']:.2f} | {where} |"
        )
    mean, median = summary_of(errors)
    return [
        *lines,
        "",
        f"Mean absolute error {mean:.1f}%, median {median:.1f}%, over {len(errors)} programs.",
    ]


def replace_section(page: Path, name: str, section: list[str]) -> None:
    """Rewrites what lies between `page`'s markers for `name`."""
    text = page.read_text()
    start = f"<!-- accuracy: {name} -->"
    head, found, rest = text.partition(start + "\n")
    if not found or END not in rest:
        raise SystemExit(f"{page} has no {start} ... {END} section")
    _, _, tail = rest.partition(END)
    page.write_text(head + start + "\n" + "\n".join(section) + "\n" + END + tail)


def main() -> None:
    sections: list[tuple[Path, str, Callable[[], list[str]]]] = [
        (DOCS / "error.md", "gem5 base", gem5_section),
        (DOCS / "rtl-rocket.md", "rtl rocket", lambda: rtl_section("rocket")),
        (DOCS / "rtl-boom.md", "rtl boom", lambda: rtl_section("boom")),
    ]
    for page, name, build in sections:
        replace_section(page, name, build())
        print(f"updated {page.relative_to(ROOT)} ({name})")


if __name__ == "__main__":
    main()
