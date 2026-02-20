"""
CLI entry point for rvsim.

Invoked by the ``rvsim`` console script installed by pip, or directly via
``python -m rvsim``.

Usage::

    rvsim <file>                           Auto-detect mode by extension
    rvsim -f <elf> [--limit N]             Bare-metal ELF
    rvsim --kernel <Image> [--disk <img>]  Boot a kernel
    rvsim --script <script.py> [args ...]  Run a Python script
    rvsim list                             List bundled programs
"""

import argparse
import os
import pathlib
import runpy
import sys

# ── Helpers ──────────────────────────────────────────────────────────────────


def _detect_mode(filepath: str) -> str:
    """Detect execution mode from file extension.

    Returns ``"script"``, ``"binary"``, or ``"kernel"``.
    """
    _, ext = os.path.splitext(filepath)
    ext = ext.lower()
    if ext == ".py":
        return "script"
    if ext == ".elf":
        return "binary"
    return "kernel"


def _find_bundled_binaries():
    """Locate bundled binary directories.

    Returns (programs_dir, benchmarks_dir) or (None, None).
    """
    # Try relative to package location (development install)
    pkg_dir = pathlib.Path(__file__).resolve().parent  # rvsim/
    repo_root = pkg_dir.parent
    programs = repo_root / "software" / "bin" / "programs"
    benchmarks = repo_root / "software" / "bin" / "benchmarks"
    if programs.is_dir() and benchmarks.is_dir():
        return programs, benchmarks

    # Try CWD
    cwd = pathlib.Path.cwd()
    programs = cwd / "software" / "bin" / "programs"
    benchmarks = cwd / "software" / "bin" / "benchmarks"
    if programs.is_dir() and benchmarks.is_dir():
        return programs, benchmarks

    return None, None


def _cmd_list():
    """List bundled programs and benchmarks."""
    from ._cli import tag

    programs, benchmarks = _find_bundled_binaries()
    if programs is None:
        print(
            "Bundled binaries not found. Run from the rvsim repository root.",
            file=sys.stderr,
        )
        sys.exit(1)

    print(f"\n{tag('programs')}")
    for f in sorted(programs.glob("*.elf")):
        print(f"  {f.stem}")

    print(f"\n{tag('benchmarks')}")
    for f in sorted(benchmarks.glob("*.elf")):
        print(f"  {f.stem}")
    print()


def _apply_cli_overrides(sim, args):
    """Apply CLI flags that override config-file settings."""
    from .config import Config
    from .types import BranchPredictor

    if sim._config_obj is None:
        sim._config_obj = Config()

    if args.trace:
        sim._config_obj.trace = True
    if args.width is not None:
        sim._config_obj.width = args.width
    if args.bp is not None:
        bp_map = {
            "static": BranchPredictor.Static,
            "gshare": BranchPredictor.GShare,
            "tage": BranchPredictor.TAGE,
            "perceptron": BranchPredictor.Perceptron,
            "tournament": BranchPredictor.Tournament,
        }
        sim._config_obj.branch_predictor = bp_map[args.bp]()


# ── Main ─────────────────────────────────────────────────────────────────────


def main() -> None:
    # Handle 'list' subcommand before argparse
    if len(sys.argv) >= 2 and sys.argv[1] == "list":
        _cmd_list()
        return

    from importlib.metadata import version as _meta_version
    from .types import _parse_cycles

    parser = argparse.ArgumentParser(
        prog="rvsim",
        description="RISC-V cycle-accurate simulator",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "examples:\n"
            "  rvsim mandelbrot.elf                     run a bare-metal ELF\n"
            "  rvsim mandelbrot.elf --limit 5M           stop after 5M cycles\n"
            "  rvsim mandelbrot.elf --width 4 --bp tage  4-wide with TAGE predictor\n"
            "  rvsim --kernel Image --disk root.img      boot a kernel\n"
            "  rvsim experiment.py --ipc 4               run a Python script\n"
            "  rvsim list                                list bundled programs\n"
        ),
    )

    # Version
    parser.add_argument(
        "--version",
        action="version",
        version=f"rvsim {_meta_version('rvsim')}",
    )

    # Input selection (explicit flags for backward compat)
    parser.add_argument(
        "-f",
        "--file",
        dest="file_flag",
        metavar="BINARY",
        help="bare-metal ELF to execute",
    )
    parser.add_argument("--kernel", metavar="IMAGE", help="kernel image for OS boot")
    parser.add_argument(
        "--script", metavar="SCRIPT", help="Python script to run (gem5-style)"
    )

    # Configuration
    parser.add_argument("--config", metavar="FILE", help="Python config file")

    # Simulation
    parser.add_argument("--disk", metavar="IMG", help="disk image (requires --kernel)")
    parser.add_argument(
        "--dtb", metavar="DTB", help="device tree blob (requires --kernel)"
    )
    parser.add_argument(
        "--limit",
        metavar="N",
        type=_parse_cycles,
        default=None,
        help="max cycles to simulate (supports K/M/G, e.g. 5M)",
    )
    parser.add_argument(
        "--progress",
        metavar="N",
        type=_parse_cycles,
        default=0,
        help="print progress every N cycles (supports K/M/G, e.g. 500K)",
    )
    parser.add_argument(
        "--trace", action="store_true", default=False, help="enable instruction tracing"
    )

    # Stats control
    parser.add_argument(
        "--stats",
        nargs="?",
        const="all",
        default="all",
        metavar="SECTIONS",
        help="stats sections to print (comma-separated: summary,core,instruction_mix,branch,memory)",
    )
    parser.add_argument(
        "--no-stats", action="store_true", default=False, help="suppress stats output"
    )
    parser.add_argument(
        "--output-stats",
        metavar="FILE",
        default=None,
        help="write stats as JSON to FILE",
    )

    # Pipeline overrides
    parser.add_argument(
        "--width", type=int, metavar="N", default=None, help="pipeline width override"
    )
    parser.add_argument(
        "--bp",
        choices=["static", "gshare", "tage", "perceptron", "tournament"],
        default=None,
        metavar="TYPE",
        help="branch predictor override (static, gshare, tage, perceptron, tournament)",
    )

    # Positional: file + optional script args
    parser.add_argument("positional_args", nargs="*", help=argparse.SUPPRESS)

    # ── Parse ────────────────────────────────────────────────────────────────

    args, remaining = parser.parse_known_args()

    # Resolve input file and mode
    explicit_count = sum(1 for x in [args.file_flag, args.kernel, args.script] if x)
    if explicit_count > 1:
        parser.error("only one of -f/--file, --kernel, --script can be specified")

    if args.file_flag:
        target = args.file_flag
        mode = "binary"
        extra_args = args.positional_args + remaining
    elif args.kernel:
        target = args.kernel
        mode = "kernel"
        extra_args = args.positional_args + remaining
    elif args.script:
        target = args.script
        mode = "script"
        extra_args = args.positional_args + remaining
    elif args.positional_args:
        target = args.positional_args[0]
        mode = _detect_mode(target)
        extra_args = args.positional_args[1:] + remaining
    else:
        parser.error(
            "no input file specified\nusage: rvsim <file> [options]\n       rvsim list"
        )
        return  # unreachable, but keeps type checker happy

    # Validation
    if mode != "script" and extra_args:
        parser.error(f"unrecognized arguments: {' '.join(extra_args)}")
    if args.disk and mode != "kernel":
        parser.error("--disk requires --kernel or a kernel image")
    if args.dtb and mode != "kernel":
        parser.error("--dtb requires --kernel or a kernel image")

    # Resolve stats sections
    if args.no_stats:
        stats_sections = None
    elif args.stats and args.stats != "all":
        stats_sections = [s.strip() for s in args.stats.split(",")]
    else:
        stats_sections = []  # empty = all sections

    # ── Execute ──────────────────────────────────────────────────────────────

    if mode == "script":
        sys.argv = [target] + extra_args
        runpy.run_path(target, run_name="__main__")

    elif mode == "kernel":
        from .objects import Simulator

        sim = Simulator()
        if args.config:
            sim = sim.config(args.config)
        _apply_cli_overrides(sim, args)
        sim = sim.kernel(target).kernel_mode()
        if args.disk:
            sim = sim.disk(args.disk)
        if args.dtb:
            sim = sim.dtb(args.dtb)
        sys.exit(
            sim.run(
                limit=args.limit,
                progress=args.progress,
                stats_sections=stats_sections,
                output_stats=args.output_stats,
            )
        )

    else:  # binary
        from .objects import Simulator

        sim = Simulator()
        if args.config:
            sim = sim.config(args.config)
        _apply_cli_overrides(sim, args)
        sim = sim.binary(target)
        sys.exit(
            sim.run(
                limit=args.limit,
                progress=args.progress,
                stats_sections=stats_sections,
                output_stats=args.output_stats,
            )
        )


if __name__ == "__main__":
    main()
