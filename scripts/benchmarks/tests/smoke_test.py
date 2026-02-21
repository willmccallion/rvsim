"""Minimal smoke test: one benchmark with base config. Run: sim script scripts/tests/smoke_test.py"""

import os
import sys

_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_root = os.path.dirname(os.path.dirname(_scripts))

from rvsim import Config, Environment


def main():
    binary = os.path.join(_root, "software", "bin", "benchmarks", "qsort.elf")
    if not os.path.exists(binary):
        print(f"Skip: binary not found: {binary}")
        return 0

    print(f"[smoke] Running {os.path.basename(binary)}...")
    res = Environment(binary=binary, config=Config(uart_quiet=True)).run()

    print(f"\nResult: {'SUCCESS' if res.ok else 'FAILURE'} (exit {res.exit_code})")
    print(res.stats.query("ipc|cycles"))
    return 0 if res.ok else 1


if __name__ == "__main__":
    sys.exit(main())
