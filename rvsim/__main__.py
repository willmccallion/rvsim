"""
CLI entry point for rvsim.

Usage::

    rvsim <file> [options]   Run an ELF binary, kernel image, or Python script
    rvsim list               List bundled programs
"""

import argparse
import os
import pathlib
import runpy
import sys

# ── Helpers ───────────────────────────────────────────────────────────────────

_PROGRAM_DESCRIPTIONS = {
    "chess": "alpha-beta chess engine, searches to fixed depth",
    "fib": "fibonacci sequence, simple ALU benchmark",
    "life": "Conway's Game of Life, memory-bound grid update",
    "mandelbrot": "Mandelbrot set renderer, floating-point heavy",
    "maze": "recursive maze generator, branch-heavy",
    "merge_sort": "merge sort on random integers",
    "qsort": "quicksort, cache and branch benchmark",
    "raytracer": "software ray tracer, FP and memory intensive",
    "sand": "falling sand simulation",
    "sort": "sorting benchmark",
    "twentyfortyeight": "2048 game logic",
}


def _detect_mode(filepath: str) -> str:
    _, ext = os.path.splitext(filepath)
    ext = ext.lower()
    if ext == ".py":
        return "script"
    if ext == ".elf":
        return "binary"
    return "kernel"


def _find_bundled_binaries():
    pkg_dir = pathlib.Path(__file__).resolve().parent
    repo_root = pkg_dir.parent
    programs = repo_root / "software" / "bin" / "programs"
    benchmarks = repo_root / "software" / "bin" / "benchmarks"
    if programs.is_dir() and benchmarks.is_dir():
        return programs, benchmarks
    cwd = pathlib.Path.cwd()
    programs = cwd / "software" / "bin" / "programs"
    benchmarks = cwd / "software" / "bin" / "benchmarks"
    if programs.is_dir() and benchmarks.is_dir():
        return programs, benchmarks
    return None, None


def _cmd_list():
    from ._cli import BOLD, DIM, RESET, TEAL

    programs, benchmarks = _find_bundled_binaries()
    if programs is None:
        print(
            "Bundled binaries not found. Run from the rvsim repository root.",
            file=sys.stderr,
        )
        sys.exit(1)

    is_tty = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()

    def section(name):
        if is_tty:
            return f"\n{BOLD}{TEAL}[{name}]{RESET}"
        return f"\n[{name}]"

    def row(stem, desc):
        if is_tty:
            return f"  {BOLD}{stem:<18}{RESET}{DIM}{desc}{RESET}"
        return f"  {stem:<18}{desc}"

    print(section("programs"))
    for f in sorted(programs.glob("*.elf")):
        desc = _PROGRAM_DESCRIPTIONS.get(f.stem, "")
        print(row(f.stem, desc))

    print(section("benchmarks"))
    for f in sorted(benchmarks.glob("*.elf")):
        print(row(f.stem, ""))

    print()


def _print_help() -> None:
    from rich.console import Console
    from rich.padding import Padding
    from rich.table import Table
    from rich.text import Text

    console = Console()

    console.print()
    console.print(
        "  [bold cyan]rvsim[/] [dim]—[/] RISC-V cycle-accurate simulator",
        highlight=False,
    )
    console.print()

    # Usage
    console.print("  [bold]Usage[/]", highlight=False)
    console.print(
        "    [cyan]rvsim[/] [green]<file>[/] [dim][[/][yellow]options[/][dim]][/]",
        highlight=False,
    )
    console.print("    [cyan]rvsim[/] [green]list[/]", highlight=False)
    console.print()

    # Mode detection
    console.print(
        "  [bold]Modes[/]  [dim](auto-detected from file extension)[/]", highlight=False
    )
    mode_table = Table(show_header=False, box=None, padding=(0, 2))
    mode_table.add_column(style="green", width=8)
    mode_table.add_column(style="dim")
    mode_table.add_row(".elf", "bare-metal ELF binary")
    mode_table.add_row(".py", "Python script — use the rvsim API for full control")
    mode_table.add_row("other", "kernel image")
    console.print(Padding(mode_table, (0, 2)))
    console.print()

    # Options
    console.print("  [bold]Options[/]", highlight=False)
    opt_table = Table(show_header=False, box=None, padding=(0, 2))
    opt_table.add_column(style="yellow", no_wrap=True)
    opt_table.add_column(style="dim")
    opt_table.add_row(
        "--watch", "live dashboard — IPC, cache hit rates, branch accuracy, stalls"
    )
    opt_table.add_row(
        "--limit [cyan]N[/cyan]", "stop after N cycles  [dim](e.g. 5M, 500K, 1G)[/dim]"
    )
    opt_table.add_row("--no-stats", "run without printing the stats table")
    opt_table.add_row("--quiet", "suppress all output, including program stdout")
    opt_table.add_row("--json [cyan]FILE[/cyan]", "write stats as JSON to FILE")
    console.print(Padding(opt_table, (0, 2)))
    console.print()

    # Examples
    console.print("  [bold]Examples[/]", highlight=False)
    ex_table = Table(show_header=False, box=None, padding=(0, 2))
    ex_table.add_column(style="cyan", no_wrap=True)
    ex_table.add_column(style="dim")
    ex_table.add_row("rvsim mandelbrot.elf", "run, print stats on exit")
    ex_table.add_row("rvsim mandelbrot.elf --watch", "live dashboard while running")
    ex_table.add_row("rvsim mandelbrot.elf --limit 5M", "stop after 5 million cycles")
    ex_table.add_row("rvsim mandelbrot.elf --quiet", "suppress all output")
    ex_table.add_row("rvsim mandelbrot.elf --json out.json", "save stats to JSON")
    ex_table.add_row("rvsim experiment.py", "run a Python script via the rvsim API")
    ex_table.add_row("rvsim list", "list bundled programs and benchmarks")
    console.print(Padding(ex_table, (0, 2)))
    console.print()

    # Tip
    console.print(
        "  [dim]For pipeline config, cache tuning, sweeps — write a .py script and pass it here.[/]",
        highlight=False,
    )
    console.print()


# ── Main ──────────────────────────────────────────────────────────────────────


def main() -> None:
    if len(sys.argv) == 1 or (len(sys.argv) == 2 and sys.argv[1] in ("-h", "--help")):
        _print_help()
        sys.exit(0)

    if len(sys.argv) >= 2 and sys.argv[1] == "list":
        _cmd_list()
        return

    from importlib.metadata import version as _meta_version
    from .types import _parse_cycles

    parser = argparse.ArgumentParser(
        prog="rvsim",
        description=(
            "rvsim — RISC-V cycle-accurate simulator\n"
            "\n"
            "Run a bare-metal ELF, kernel image, or Python script.\n"
            "The mode is auto-detected from the file extension:\n"
            "  .elf  → bare-metal binary\n"
            "  .py   → Python script (use the rvsim Python API for full control)\n"
            "  other → kernel image\n"
            "\n"
            "For anything beyond a quick one-off run — configuring the pipeline,\n"
            "cache hierarchy, branch predictor, sweeps — write a Python script\n"
            "and pass it here instead."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "examples:\n"
            "  rvsim mandelbrot.elf               run with default config, print stats\n"
            "  rvsim mandelbrot.elf --watch        live dashboard (IPC, cache, branch, stalls)\n"
            "  rvsim mandelbrot.elf --limit 5M     stop after 5 million cycles\n"
            "  rvsim mandelbrot.elf --no-stats     run without printing stats\n"
            "  rvsim mandelbrot.elf --quiet        suppress all output including program stdout\n"
            "  rvsim mandelbrot.elf --json out.json  save stats to JSON\n"
            "  rvsim experiment.py                 run a Python script via the rvsim API\n"
            "  rvsim list                          list bundled programs and benchmarks\n"
        ),
    )

    parser.add_argument(
        "--version", action="version", version=f"rvsim {_meta_version('rvsim')}"
    )
    parser.add_argument(
        "--limit",
        metavar="N",
        type=_parse_cycles,
        default=None,
        help="stop after N cycles (e.g. 5M, 500K)",
    )
    parser.add_argument(
        "--watch",
        action="store_true",
        default=False,
        help="live dashboard while the simulation runs",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        default=False,
        help="suppress all output (program stdout and stats)",
    )
    parser.add_argument(
        "--no-stats",
        action="store_true",
        default=False,
        help="suppress stats table but still show program output",
    )
    parser.add_argument(
        "--json",
        metavar="FILE",
        default=None,
        help="write stats as JSON to FILE",
    )
    parser.add_argument("positional_args", nargs="*", help=argparse.SUPPRESS)

    args, remaining = parser.parse_known_args()

    if not args.positional_args:
        _print_help()
        sys.exit(0)

    target = args.positional_args[0]
    mode = _detect_mode(target)
    extra_args = args.positional_args[1:] + remaining

    if mode != "script" and extra_args:
        parser.error(f"unrecognized arguments: {' '.join(extra_args)}")

    # ── Execute ───────────────────────────────────────────────────────────────

    if mode == "script":
        sys.argv = [target] + extra_args
        runpy.run_path(target, run_name="__main__")
        return

    from .config import Config
    from .objects import Simulator

    cfg = Config()
    if args.quiet:
        cfg.uart_quiet = True
    elif args.watch:
        # Redirect program output to stderr so Live doesn't see stdout being
        # written mid-render (which causes the duplicate frame).
        cfg.uart_to_stderr = True
    elif mode == "kernel":
        cfg.uart_to_stderr = True

    sim = Simulator().config(cfg)

    if mode == "kernel":
        sim = sim.kernel(target)
    else:
        sim = sim.binary(target)

    if args.watch:
        import io
        from ._watch import run_watch

        # Suppress setup chatter so it doesn't appear above the dashboard.
        _real_stderr = sys.stderr
        sys.stderr = io.StringIO()
        try:
            cpu = sim.build()
        finally:
            sys.stderr = _real_stderr
        print_stats = not args.quiet and not args.no_stats
        exit_code = run_watch(
            cpu,
            limit=args.limit,
            binary=os.path.basename(target),
            print_stats=print_stats,
        )
    else:
        stats_sections = None if (args.quiet or args.no_stats) else []
        cpu = sim.build()
        exit_code = cpu.run(limit=args.limit, stats_sections=stats_sections)

    if args.json and exit_code is not None:
        import json

        with open(args.json, "w") as f:
            json.dump(dict(cpu.stats), f, indent=2)

    sys.exit(exit_code if exit_code is not None else 1)


if __name__ == "__main__":
    main()
