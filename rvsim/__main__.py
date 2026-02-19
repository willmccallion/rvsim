"""
CLI entry point for rvsim.

Invoked by the ``rvsim`` console script installed by pip, or directly via
``python -m rvsim``.

Usage::

    rvsim -f <binary>
    rvsim --kernel <Image> [--disk <img>] [--dtb <dtb>]
    rvsim --script <script.py> [args ...]
"""

import argparse
import runpy
import sys


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="rvsim",
        description="RISC-V cycle-accurate simulator",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n"
            "  rvsim -f hello.elf\n"
            "  rvsim --kernel Image --disk root.img\n"
            "  rvsim --script experiment.py --ipc 4\n"
        ),
    )

    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "-f", "--file", metavar="BINARY", help="Bare-metal ELF/binary to execute"
    )
    mode.add_argument("--kernel", metavar="IMAGE", help="Kernel image for OS boot")
    mode.add_argument(
        "--script", metavar="SCRIPT", help="Python script to run (gem5-style)"
    )

    parser.add_argument("--disk", metavar="IMG", help="Disk image (requires --kernel)")
    parser.add_argument(
        "--dtb", metavar="DTB", help="Device tree blob (requires --kernel)"
    )
    parser.add_argument(
        "script_args",
        nargs=argparse.REMAINDER,
        help="Arguments forwarded to --script as sys.argv[1:]",
    )

    args = parser.parse_args()

    if args.disk and not args.kernel:
        parser.error("--disk requires --kernel")
    if args.dtb and not args.kernel:
        parser.error("--dtb requires --kernel")

    if args.script:
        # gem5-style: run the user script with sys.argv set correctly.
        # The rvsim package is already importable (we're inside it).
        sys.argv = [args.script] + args.script_args
        runpy.run_path(args.script, run_name="__main__")

    elif args.kernel:
        from .objects import Simulator

        sim = Simulator().kernel(args.kernel).kernel_mode()
        if args.disk:
            sim = sim.disk(args.disk)
        if args.dtb:
            sim = sim.dtb(args.dtb)
        sys.exit(sim.run())

    else:
        from .objects import Simulator

        sys.exit(Simulator().binary(args.file).run())


if __name__ == "__main__":
    main()
