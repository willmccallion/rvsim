"""
Run one benchmark on M1 config; print stats via .query().
Edit scripts/m1/config.py to change the machine.

  sim script scripts/m1/run.py [binary]
"""

import os
import sys

_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts)

from m1.config import m1_config
from rvsim import Environment

_root = os.path.dirname(_scripts)
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.elf")


def main():
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".elf") else BINARY
    )
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    config = m1_config()
    env = Environment(binary=binary, config=config)
    result = env.run(quiet=True)
    print("M1 (one run):", binary)
    print("  exit_code:", result.exit_code)
    print("  IPC:", result.stats.get("ipc"))
    print("  .query('miss'):")
    print(result.stats.query("miss"))
    print("  .query('branch'):")
    print(result.stats.query("branch"))


if __name__ == "__main__":
    main()
