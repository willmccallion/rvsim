"""
Run one benchmark on P550 config; print stats via .query().
Edit scripts/p550/config.py to change the machine.

  sim script scripts/p550/run.py [binary]
"""

import os
import sys

_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts)

from p550.config import p550_config
from rvsim import Environment

_root = os.path.dirname(_scripts)
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.elf")


def main():
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".elf") else BINARY
    )
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    config = p550_config()
    env = Environment(binary=binary, config=config)
    result = env.run(quiet=True)
    print("P550 (one run):", binary)
    print("  exit_code:", result.exit_code)
    if result.exit_code == -1:
        err = result.stats.get("error")
        if err:
            print("  error:", err)
    print("  IPC:", result.stats.get("ipc"))
    print("  .query('miss'):")
    print(result.stats.query("miss"))
    print("  .query('branch'):")
    print(result.stats.query("branch"))


if __name__ == "__main__":
    main()
