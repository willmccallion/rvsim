"""
Run one benchmark on P550 and M1 configs, then show stat differences using .query().
Run: sim script scripts/tests/compare_p550_m1.py [binary]
"""
import sys
import os
_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts)

from riscv_sim import Environment, run_experiment
from p550.config import p550_config
from m1.config import m1_config

_root = os.path.dirname(_scripts)
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.bin")


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".bin") else BINARY
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    name = os.path.basename(binary)

    # P550
    env_p550 = Environment(binary=binary, config=p550_config(branch_predictor="TAGE"))
    r550 = run_experiment(env_p550, quiet=True)
    # M1
    env_m1 = Environment(binary=binary, config=m1_config(branch_predictor="TAGE"))
    r_m1 = run_experiment(env_m1, quiet=True)

    print("Compare P550 vs M1:", name)
    print()

    # Summary
    print("Summary:")
    print("         P550      M1       diff")
    print("  cycles ", r550.stats.get("cycles", 0), " ", r_m1.stats.get("cycles", 0),
          " ", r_m1.stats.get("cycles", 0) - r550.stats.get("cycles", 0))
    print("  ipc    ", f"{r550.stats.get('ipc', 0):.4f}", "  ", f"{r_m1.stats.get('ipc', 0):.4f}",
          " ", f"{r_m1.stats.get('ipc', 0) - r550.stats.get('ipc', 0):.4f}")
    print()

    # Query "miss" – cache/branch misses
    miss550 = r550.stats.query("miss")
    miss_m1 = r_m1.stats.query("miss")
    print(".query('miss') – P550:")
    print(miss550)
    print(".query('miss') – M1:")
    print(miss_m1)
    print("Difference (M1 - P550):")
    for k in sorted(set(miss550) | set(miss_m1)):
        v550 = miss550.get(k, 0)
        v_m1 = miss_m1.get(k, 0)
        if isinstance(v550, (int, float)) and isinstance(v_m1, (int, float)):
            print(f"  {k}: {v_m1 - v550}")
    print()

    # Query "branch"
    bp550 = r550.stats.query("branch")
    bp_m1 = r_m1.stats.query("branch")
    print(".query('branch') – P550 branch_accuracy_pct:", bp550.get("branch_accuracy_pct"))
    print(".query('branch') – M1  branch_accuracy_pct:", bp_m1.get("branch_accuracy_pct"))


if __name__ == "__main__":
    main()
