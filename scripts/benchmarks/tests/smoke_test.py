"""Minimal smoke test: one benchmark with base config. Run: sim script scripts/tests/smoke_test.py"""

import sys
from pathlib import Path

from rvsim import Config, Environment

_ROOT = Path(__file__).resolve().parent.parent.parent.parent


def main():
    binary = str(_ROOT / "software" / "bin" / "benchmarks" / "qsort.elf")
    if not Path(binary).exists():
        print(f"Skip: binary not found: {binary}")
        return 0

    print(f"[smoke] Running {Path(binary).name}...")
    res = Environment(binary=binary, config=Config(uart_quiet=True)).run()

    print(f"\nResult: {'SUCCESS' if res.ok else 'FAILURE'} (exit {res.exit_code})")
    print(res.stats.query("ipc|cycles"))
    return 0 if res.ok else 1


if __name__ == "__main__":
    sys.exit(main())
