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
from riscv_sim import Environment, run_experiment

_root = os.path.dirname(_scripts)
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.bin")


def main():
    """
    Main execution entry point for running a single P550 simulation experiment.

    This function determines the target binary from command-line arguments or defaults,
    configures the P550 environment with a TAGE branch predictor, executes the
    simulation, and prints performance metrics including IPC and specific
    statistics for cache misses and branch predictions.
    """
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".bin") else BINARY
    )
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    config = p550_config(branch_predictor="TAGE")
    env = Environment(binary=binary, config=config)
    result = run_experiment(env, quiet=True)
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
