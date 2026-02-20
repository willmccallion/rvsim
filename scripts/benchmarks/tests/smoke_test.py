"""Minimal smoke test: one benchmark with base config. Run: sim script scripts/tests/smoke_test.py"""

import os
import sys

_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(_root, "python"))

from rvsim import Config, Environment


def main():
    root = _root
    binary = os.path.join(root, "../" "software", "bin", "benchmarks", "qsort.elf")
    if not os.path.exists(binary):
        print("Skip: binary not found:", binary)
        return 0
    env = Environment(binary=binary, config=Config())
    result = env.run(quiet=True)
    print(
        "smoke_test: exit_code=%s ipc=%s" % (result.exit_code, result.stats.get("ipc"))
    )
    return 0 if result.exit_code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
