"""``rvsim bench``: the benchmark suite on the bundled Linux image.

Boots once (cached), switches to the configuration under study, and
measures each benchmark as a region of its own.
"""

from __future__ import annotations

import argparse
import datetime
import json
import math
import os
import sys
from typing import Any

from .. import presets
from ..config import load_config
from ..presets import INTERCONNECTS
from ..session import Region, Session
from ..session.stops import LOGIN_SHELL

#: Shell commands on the bundled image, sized to finish in minutes of
#: detailed simulation.
LINUX_BENCHMARKS: dict[str, str] = {
    "coremark": "coremark 0x0 0x0 0x66 20 7 1 2000",
    "dhrystone": "dhrystone 200000",
    "whetstone": "whetstone 20",
    "stream": "stream",
    "mbw": "mbw -q -n 1 4",
    "lat_mem_rd": "lat_mem_rd -P 1 -N 1 8 128",
    "stress-ng": "stress-ng --cpu 0 --cpu-method matrixprod --cpu-ops 64 --quiet",
}

_COLUMNS = (
    "cycles",
    "instructions",
    "ipc",
    "branch_mpki",
    "l1d_mpki",
    "l2_mpki",
    "llc_mpki",
)


def headline(region: Region) -> dict[str, float]:
    """The metrics the summary table shows for a region."""
    stats = region.stats
    kilo = region.instructions / 1000 if region.instructions else math.nan

    def per_kilo(pattern: str) -> float:
        return stats.query(pattern).sum() / kilo

    return {
        "cycles": region.cycles,
        "instructions": region.instructions,
        "ipc": region.ipc,
        "branch_mpki": per_kilo("core*.bp.committed.mispredicts"),
        "l1d_mpki": per_kilo("core*.cache.l1d.misses"),
        "l2_mpki": per_kilo("core*.cache.l2.misses"),
        "llc_mpki": per_kilo("llc.misses"),
        "dram_reads": stats.query("memctrl*.ch*.sc*.reads").sum(),
        "dram_writes": stats.query("memctrl*.ch*.sc*.writes").sum(),
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="rvsim bench",
        description=(
            "Run benchmarks inside Linux on a configuration. The boot to a "
            "shell is fast-forwarded once and cached; each benchmark is then "
            "measured on the chosen configuration as its own stats region."
        ),
    )
    parser.add_argument(
        "benchmarks",
        nargs="*",
        metavar="NAME",
        help=f"any of {', '.join(LINUX_BENCHMARKS)} (default: all)",
    )
    core = parser.add_mutually_exclusive_group()
    core.add_argument(
        "--preset",
        choices=["basic", "fast"],
        default="fast",
        help="core configuration (default fast)",
    )
    core.add_argument(
        "--config",
        metavar="FILE",
        help="Python config file whose core is placed in the Linux system",
    )
    parser.add_argument("--harts", type=int, default=8, help="harts (default 8)")
    parser.add_argument("--memory", choices=["ddr5", "dram"], default="ddr5")
    parser.add_argument("--interconnect", choices=sorted(INTERCONNECTS), default="mesh")
    parser.add_argument(
        "--warm",
        action="store_true",
        help="run each benchmark once unmeasured before measuring it",
    )
    parser.add_argument(
        "--json", metavar="FILE", help="write every region's stats and output to FILE"
    )
    parser.add_argument(
        "--no-cache", action="store_true", help="boot even if a cached boot exists"
    )
    parser.add_argument("--echo", action="store_true", help="show the guest console")
    parser.add_argument(
        "--list", action="store_true", help="list the benchmarks and exit"
    )
    return parser


def main(argv: list[str]) -> int:
    parser = _parser()
    args = parser.parse_args(argv)
    if args.list:
        for name, command in LINUX_BENCHMARKS.items():
            print(f"{name:12} {command}")
        return 0
    names = args.benchmarks or list(LINUX_BENCHMARKS)
    unknown = [name for name in names if name not in LINUX_BENCHMARKS]
    if unknown:
        parser.error(f"unknown benchmarks {unknown}; see --list")
    if args.harts < 1:
        parser.error("--harts must be at least 1")

    core = load_config(args.config) if args.config else presets.PRESETS[args.preset]()
    config = presets.linux(
        args.harts, memory=args.memory, interconnect=args.interconnect, core=core
    )
    session = Session.linux(
        config, progress=True, echo=sys.stderr if args.echo else False
    )
    session.fast_forward(until=LOGIN_SHELL, cache=not args.no_cache)

    regions: dict[str, Region] = {}
    for name in names:
        command = LINUX_BENCHMARKS[name]
        if args.warm:
            session.warm_up(command=command)
        region = session.measure(command, name=name)
        regions[name] = region
        _say(
            f"{name}: exit {region.exit_code}, {region.instructions:,} instructions, "
            f"IPC {region.ipc:.3f}, {region.host_seconds:.0f}s"
        )

    _print_table(regions)
    if args.json:
        _write_json(args.json, args, regions)
    return 0 if all(region.exit_code == 0 for region in regions.values()) else 1


def _print_table(regions: dict[str, Region]) -> None:
    from rich.console import Console
    from rich.table import Table

    table = Table(title="Benchmarks", title_justify="left")
    table.add_column("benchmark", style="bold")
    table.add_column("exit", justify="right")
    for column in _COLUMNS:
        table.add_column(column, justify="right")
    for name, region in regions.items():
        metrics = headline(region)
        table.add_row(
            name,
            str(region.exit_code),
            *(_format(column, metrics[column]) for column in _COLUMNS),
        )
    Console().print(table)


def _format(column: str, value: float) -> str:
    if column in ("cycles", "instructions"):
        return f"{int(value):,}"
    return f"{value:.3f}"


def _write_json(
    path: str, args: argparse.Namespace, regions: dict[str, Region]
) -> None:
    record: dict[str, Any] = {
        "created": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "core": args.config or args.preset,
        "harts": args.harts,
        "memory": args.memory,
        "interconnect": args.interconnect,
        "warm": args.warm,
        "results": {
            name: {**region.to_dict(), "headline": headline(region)}
            for name, region in regions.items()
        },
    }
    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, exist_ok=True)
    with open(path, "w") as f:
        json.dump(record, f, indent=1)
    _say(f"wrote {path}")


def _say(message: str) -> None:
    sys.stderr.write(f"[rvsim] {message}\n")
    sys.stderr.flush()
