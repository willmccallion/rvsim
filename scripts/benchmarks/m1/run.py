"""
Run one benchmark on M1 config; print stats via .query().
Edit scripts/m1/config.py to change the machine.

  sim script scripts/m1/run.py [binary]
"""

import importlib.util
import os
import sys
from pathlib import Path

from rvsim import Environment

_DIR = Path(__file__).resolve().parent
_ROOT = _DIR.parent.parent.parent
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.elf")

# Load sibling config.py without sys.path manipulation
_spec = importlib.util.spec_from_file_location("m1_config", _DIR / "config.py")
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)
m1_config = _mod.m1_config


def main():
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".elf") else BINARY
    )
    if not os.path.isabs(binary):
        binary = str(_ROOT / binary)
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
