"""
Run one benchmark on P550 and M1 configs, then show stat differences using .query().
Run: sim script scripts/tests/compare_p550_m1.py [binary]
"""

import os
import sys

_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, _scripts)

from m1.config import m1_config
from p550.config import p550_config

from rvsim import Environment, Result, Stats

_root = os.path.dirname(os.path.dirname(_scripts))
BINARY = os.path.join("software", "bin", "benchmarks", "qsort.elf")


def main():
    binary = (
        sys.argv[1] if len(sys.argv) > 1 and sys.argv[1].endswith(".elf") else BINARY
    )
    if not os.path.isabs(binary):
        binary = os.path.join(_root, binary)
    name = os.path.basename(binary)

    # Run simulations
    r550 = Environment(binary=binary, config=p550_config().replace(uart_quiet=True)).run()
    r_m1 = Environment(binary=binary, config=m1_config().replace(uart_quiet=True)).run()

    print(f"Comparison: {name}\n")

    # 1. High-level summary and speedup
    Result.compare({"P550": r550, "M1": r_m1}, baseline="P550", col_header="config")
    print()

    # 2. Detailed miss comparison
    print(
        Stats.tabulate(
            {"P550": r550.stats.query("miss"), "M1": r_m1.stats.query("miss")},
            title="Cache & Branch Misses",
        )
    )
    print()

    # 3. Branch prediction details
    print(
        Stats.tabulate(
            {"P550": r550.stats.query("branch"), "M1": r_m1.stats.query("branch")},
            title="Branch Predictor Performance",
        )
    )


if __name__ == "__main__":
    main()
