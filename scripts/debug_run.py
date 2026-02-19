"""
Debug script: run a binary for a limited number of cycles and dump state.

Usage:
    python scripts/debug_run.py software/bin/programs/qsort.bin
    python scripts/debug_run.py software/bin/programs/qsort.bin --cycles 1000000
    python scripts/debug_run.py software/bin/programs/qsort.bin --disasm
    python scripts/debug_run.py software/bin/programs/qsort.bin --disasm --at 0x80000024 --limit 10
"""

import argparse
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from rvsim import Config, Environment, run_experiment, reg_name, Disassemble


def main():
    parser = argparse.ArgumentParser(description="Debug run a RISC-V binary")
    parser.add_argument("binary", help="Path to binary")
    parser.add_argument(
        "--cycles", type=int, default=100000, help="Max cycles (default 100000)"
    )
    parser.add_argument(
        "--disasm", action="store_true", help="Disassemble instead of running"
    )
    parser.add_argument(
        "--at",
        type=lambda x: int(x, 0),
        default=None,
        help="Disassemble starting address",
    )
    parser.add_argument(
        "--limit", type=int, default=40, help="Number of instructions to disassemble"
    )
    args = parser.parse_args()

    if args.disasm:
        d = Disassemble().binary(args.binary)
        if args.at is not None:
            d = d.at(args.at, count=args.limit)
        else:
            d = d.limit(args.limit)
        d.print()
        return

    config = Config()
    print(f"Config: {config}")
    print(f"  initial_sp={config.initial_sp}")
    print()

    env = Environment(binary=args.binary, config=config)
    result = run_experiment(env, quiet=False)
    print(f"\nExit code: {result.exit_code}")
    print(f"Wall time: {result.wall_time_sec:.3f}s")
    print(f"\nStats:")
    for k, v in sorted(result.stats.items()):
        print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
