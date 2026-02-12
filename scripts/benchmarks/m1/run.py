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
from riscv_sim import Environment, run_experiment

_root = os.path.dirname(_scripts)
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.bin")


def main():
    """
    Main entry point for running a single M1 simulation experiment.

    This function determines the target binary from command-line arguments or defaults,
    configures the M1 simulation with a TAGE branch predictor, executes the
    experiment, and prints performance metrics including IPC and branch statistics.
    """
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".bin") else BINARY
    )
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    config = m1_config(branch_predictor="TAGE")
    env = Environment(binary=binary, config=config)
    result = run_experiment(env, quiet=True)
    print("M1 (one run):", binary)
    print("  exit_code:", result.exit_code)
    print("  IPC:", result.stats.get("ipc"))
    print("  .query('miss'):")
    print(result.stats.query("miss"))
    print("  .query('branch'):")
    print(result.stats.query("branch"))


if __name__ == "__main__":
    main()
