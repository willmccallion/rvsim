"""
CLI entry point for rvsim.

Usage::

    rvsim <file> [options]   Run an ELF binary, kernel image, or Python script
    rvsim bench [names]      Run benchmarks inside Linux on a configuration
    rvsim list               List bundled programs
"""

import argparse
import os
import pathlib
import runpy
import sys
from dataclasses import dataclass, field
from typing import Any

from ..config import ConsoleMode
from ..config._units import _parse_cycles
from ..presets import PRESETS
from ..simulator import DebugEnd

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
    pkg_dir = pathlib.Path(__file__).resolve().parent.parent
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
    from .._ansi import BOLD, DIM, RESET, TEAL, stdout_is_tty

    programs, benchmarks = _find_bundled_binaries()
    if programs is None:
        print(
            "Bundled binaries not found. Run from the rvsim repository root.",
            file=sys.stderr,
        )
        sys.exit(1)

    is_tty = stdout_is_tty()

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


@dataclass(frozen=True)
class _Option:
    """A command-line option: its flags, the argparse settings it takes,
    and the help both the rich help and ``--help`` show."""

    flags: tuple[str, ...]
    help: str
    settings: dict[str, Any] = field(default_factory=dict)

    @property
    def shown(self) -> str:
        """The option as the rich help lists it, with its value's name."""
        metavar = self.settings.get("metavar")
        return f"{self.flags[0]} [cyan]{metavar}[/cyan]" if metavar else self.flags[0]


_OPTIONS = (
    _Option(
        ("--watch",),
        "live dashboard: IPC, cache hit rates, branch accuracy, stalls",
        {"action": "store_true"},
    ),
    _Option(
        ("--limit",),
        "stop after N cycles (e.g. 5M, 500K, 1G)",
        {"metavar": "N", "type": _parse_cycles},
    ),
    _Option(
        ("--no-stats",),
        "run without printing the stats table",
        {"action": "store_true"},
    ),
    _Option(
        ("--quiet",),
        "suppress all output, including the program's",
        {"action": "store_true"},
    ),
    _Option(
        ("--config",),
        "Python config file, which exports a Config or a function returning one",
        {"metavar": "FILE"},
    ),
    _Option(
        ("--preset",),
        f"built-in config: {', '.join(PRESETS)}",
        {"choices": list(PRESETS), "metavar": "NAME"},
    ),
    _Option(("--json",), "write the run's stats as JSON to FILE", {"metavar": "FILE"}),
    _Option(
        ("--harts",),
        "run with N harts (overrides the config's hart_count)",
        {"metavar": "N", "type": int},
    ),
    _Option(
        ("--env",),
        "an environment entry for a Linux user program; repeatable",
        {"metavar": "NAME=VALUE", "action": "append", "default": []},
    ),
    _Option(
        ("--program",),
        "another Linux user program, with its arguments (shell-split), on its own hart; repeatable",
        {"metavar": "'BINARY [ARG...]'", "action": "append", "default": []},
    ),
    _Option(
        ("--mode",),
        "run the ELF as a Linux user program (se) or a bare-metal image (bare); by default an "
        "entry below RAM with no tohost symbol means se",
        {"choices": ["se", "bare"], "metavar": "se|bare"},
    ),
    _Option(
        ("--script",),
        "the file the guest's readfile pseudo-operation reads",
        {"metavar": "FILE"},
    ),
    _Option(
        ("--outdir",),
        "the directory the guest's writefile pseudo-operation writes into",
        {"metavar": "DIR"},
    ),
    _Option(
        ("--gdb",),
        "wait for GDB on this TCP port and run under it (in gdb: target remote :PORT)",
        {"metavar": "PORT", "type": int},
    ),
    _Option(
        ("--pipeline-trace",),
        "write where every instruction is each cycle, as a Kanata log for Konata",
        {"metavar": "FILE"},
    ),
)

_MODES = (
    (".elf", "static Linux user program (SE mode) or bare-metal image"),
    (".py", "Python script: use the rvsim API for full control"),
    ("other", "kernel image"),
)

_EXAMPLES = (
    ("rvsim mandelbrot.elf", "run, print stats on exit"),
    ("rvsim prog.elf -- a b", "run a Linux user program with arguments"),
    ("rvsim mandelbrot.elf --watch", "live dashboard while running"),
    ("rvsim mandelbrot.elf --limit 5M", "stop after 5 million cycles"),
    ("rvsim mandelbrot.elf --quiet", "suppress all output"),
    ("rvsim mandelbrot.elf --json out.json", "save stats to JSON"),
    ("rvsim qsort.elf --preset fast", "run on the M4-class preset"),
    ("rvsim qsort.elf --config p550.py", "run with a custom pipeline config"),
    ("rvsim spinlock.elf --harts 4", "run a multi-hart program on four harts"),
    ("rvsim experiment.py", "run a Python script through the rvsim API"),
    ("rvsim bench coremark stream", "benchmarks inside Linux; the boot is cached"),
    ("rvsim bench --help", "options: core config, harts, warm-up, JSON"),
    ("rvsim list", "list bundled programs and benchmarks"),
)

_TIP = (
    "For pipeline config, cache tuning and sweeps, write a .py script and pass it here."
)


def _rows(title: str, rows, styles: tuple[str, str]):
    """A borderless two-column table of ``rows`` under ``title``."""
    from rich.table import Table

    table = Table(
        title=title,
        title_justify="left",
        title_style="bold",
        show_header=False,
        box=None,
        padding=(0, 2),
    )
    table.add_column(style=styles[0], no_wrap=True)
    table.add_column(style="dim")
    for row in rows:
        table.add_row(*row)
    return table


def _print_help() -> None:
    from rich.console import Console
    from rich.padding import Padding

    console = Console()
    console.print(
        "\n  [bold cyan]rvsim[/] [dim]—[/] RISC-V cycle-level simulator\n",
        highlight=False,
    )
    console.print("  [bold]Usage[/]", highlight=False)
    for usage in (
        "[green]<file>[/] [dim][[/][yellow]options[/][dim]][/]",
        "[green]bench[/] [dim][[/][yellow]names[/][dim]] [[/][yellow]options[/][dim]][/]",
        "[green]list[/]",
    ):
        console.print(f"    [cyan]rvsim[/] {usage}", highlight=False)
    console.print()
    tables = (
        _rows("Modes (from the file extension)", _MODES, ("green", "dim")),
        _rows(
            "Options",
            [(option.shown, option.help) for option in _OPTIONS],
            ("yellow", "dim"),
        ),
        _rows("Examples", _EXAMPLES, ("cyan", "dim")),
    )
    for table in tables:
        console.print(Padding(table, (0, 2)))
        console.print()
    console.print(f"  [dim]{_TIP}[/]\n", highlight=False)


def _build_parser() -> argparse.ArgumentParser:
    """The parser for ``rvsim <file>``, from `_OPTIONS`, with `_MODES` and
    `_EXAMPLES` as its description and epilog."""
    from .. import __version__

    modes = "\n".join(f"  {ext:<6} {what}" for ext, what in _MODES)
    examples = "\n".join(f"  {command:<40} {what}" for command, what in _EXAMPLES)
    parser = argparse.ArgumentParser(
        prog="rvsim",
        description=f"rvsim: RISC-V cycle-level simulator\n\nThe mode comes from the file extension:\n{modes}\n\n{_TIP}",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=f"examples:\n{examples}",
    )
    parser.add_argument("--version", action="version", version=f"rvsim {__version__}")
    for option in _OPTIONS:
        parser.add_argument(*option.flags, help=option.help, **option.settings)
    parser.add_argument("positional_args", nargs="*", help=argparse.SUPPRESS)
    return parser


@dataclass(frozen=True)
class _Invocation:
    """What to run: the file, the mode its extension picks, and the
    program's own command line after it."""

    target: str
    mode: str
    program_args: list[str]


def _parse_invocation(parser, args, remaining: list[str]) -> _Invocation:
    """The file and the program's arguments, after `--` when they start
    with a dash, checked against what the mode takes."""
    target = args.positional_args[0]
    mode = _detect_mode(target)
    program_args = args.positional_args[1:] + remaining
    if program_args[:1] == ["--"]:
        program_args = program_args[1:]
    if mode == "kernel" and program_args:
        parser.error(f"unrecognized arguments: {' '.join(program_args)}")
    for entry in args.env:
        if "=" not in entry:
            parser.error(f"--env takes NAME=VALUE, not {entry!r}")
    if args.config and args.preset:
        parser.error("--config and --preset are mutually exclusive")
    if args.gdb is not None and args.watch:
        parser.error("--gdb cannot be combined with --watch")
    if args.harts is not None and args.harts < 1:
        parser.error("--harts must be at least 1")
    return _Invocation(target, mode, program_args)


def _select_config(args, mode: str):
    """The config the options name, with their hart count and console."""
    from ..config import Config, load_config

    if args.preset:
        cfg = PRESETS[args.preset]()
    elif args.config:
        cfg = load_config(args.config)
    else:
        cfg = Config()
    if args.harts is not None:
        cfg.hart_count = args.harts
    if args.quiet:
        cfg.console = ConsoleMode.QUIET
    elif args.watch or mode == "kernel":
        # The dashboard redraws stdout, and a kernel's console is a log:
        # either way the guest's output goes to stderr.
        cfg.console = ConsoleMode.STDERR
    return cfg


def _build_simulator(parser, args, cfg, run: _Invocation):
    """The simulator for `run`: a kernel, several programs, or one binary."""
    import shlex

    from ..simulator import Program, Simulator

    host_files = {"script": args.script, "outdir": args.outdir}
    if run.mode == "kernel":
        return Simulator(cfg, kernel=run.target, **host_files)
    if not args.program:
        return Simulator(
            cfg,
            binary=run.target,
            args=[run.target, *run.program_args],
            env=args.env,
            mode=args.mode,
            **host_files,
        )
    programs = [Program(run.target, run.program_args, args.env)]
    for entry in args.program:
        words = shlex.split(entry)
        if not words:
            parser.error("--program needs a binary")
        programs.append(Program(words[0], words[1:], args.env))
    if args.harts is None:
        cfg.hart_count = max(cfg.hart_count, len(programs))
    return Simulator(cfg, programs=programs, **host_files)


def _run_under_gdb(sim, port: int, *, limit) -> int | None:
    """Serves GDB on ``port``; a run GDB detaches from goes on to its exit."""
    print(
        f"waiting for gdb on 127.0.0.1:{port} (in gdb: target remote :{port})",
        file=sys.stderr,
    )
    outcome, code = sim.gdb(port)
    if outcome is DebugEnd.DETACHED:
        return sim.run(limit=limit)
    return code


def _run_watched(build, args, target: str) -> tuple[Any, int | None]:
    """Builds the simulator with setup chatter kept off the dashboard, then
    runs it under the live dashboard."""
    import io

    from .watch import run_watch

    real_stderr = sys.stderr
    sys.stderr = io.StringIO()
    try:
        sim = build()
    finally:
        sys.stderr = real_stderr
    if args.pipeline_trace:
        sim.open_pipeline_trace(args.pipeline_trace)
    exit_code = run_watch(
        sim,
        limit=args.limit,
        binary=os.path.basename(target),
        print_stats=not (args.quiet or args.no_stats),
    )
    return sim, exit_code


def _run_plain(build, args) -> tuple[Any, int | None]:
    """Builds the simulator and runs it, under GDB if asked, then prints the
    stats unless told not to."""
    sim = build()
    if args.pipeline_trace:
        sim.open_pipeline_trace(args.pipeline_trace)
    if args.gdb is not None:
        exit_code = _run_under_gdb(sim, args.gdb, limit=args.limit)
    else:
        exit_code = sim.run(limit=args.limit)
    if not (args.quiet or args.no_stats):
        print(sim.stats.summary())
    return sim, exit_code


def _write_json(path: str, sim) -> None:
    import json

    from ..stats import Stats

    with open(path, "w") as f:
        json.dump(Stats.from_core(sim.stats), f, indent=2)


def main() -> None:
    argv = sys.argv[1:]
    if not argv or argv in (["-h"], ["--help"]):
        _print_help()
        sys.exit(0)
    if argv[0] == "list":
        _cmd_list()
        return
    if argv[0] == "bench":
        from .bench import main as bench

        sys.exit(bench(argv[1:]))

    parser = _build_parser()
    args, remaining = parser.parse_known_args(argv)
    if not args.positional_args:
        _print_help()
        sys.exit(0)
    run = _parse_invocation(parser, args, remaining)
    if run.mode == "script":
        sys.argv = [run.target, *run.program_args]
        runpy.run_path(run.target, run_name="__main__")
        return

    cfg = _select_config(args, run.mode)

    def build():
        return _build_simulator(parser, args, cfg, run)

    sim, exit_code = (
        _run_watched(build, args, run.target) if args.watch else _run_plain(build, args)
    )
    if args.json:
        _write_json(args.json, sim)
    sys.exit(exit_code if exit_code is not None else 1)
