"""
Run CoreMark on the P550 config and report CoreMark/MHz.

CoreMark/MHz = (iterations * 1e6) / elapsed_cycles

This metric is frequency-independent — it equals the average IPC multiplied
by instructions-per-iteration, so it captures real pipeline efficiency.

Published reference points:
  - SiFive U74  (in-order, on Linux): ~2.63 CoreMark/MHz
  - SiFive P550 (bare-metal, OoO):    ~3.0–3.5 expected
  - Apple M1    (for scale):          ~5.0+

Note: CoreMark will print "INVALID" because the simulated run takes far less
than 10 real seconds.  Ignore that line — it refers to wall-clock time, which
is meaningless in simulation.  CoreMark/MHz from cycle count is still correct.

Usage:
    cd /path/to/rvsim
    rvsim scripts/benchmarks/p550/run_coremark.py
"""

import os
import sys

_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_repo    = os.path.dirname(os.path.dirname(_scripts))
sys.path.insert(0, _scripts)

from p550.config import p550_config
from rvsim import Environment

BINARY = os.path.join(_repo, "software/bin/benchmarks/coremark.elf")
ITERATIONS = 2000   # must match ITERATIONS in core_portme.h


def main():
    if not os.path.exists(BINARY):
        print(f"ERROR: {BINARY} not found.")
        print("Build it first:")
        print("  cd software && make coremark")
        sys.exit(1)

    print(f"Running CoreMark ({ITERATIONS} iterations) on P550 config...\n")

    result = Environment(
        binary=BINARY,
        config=p550_config(),
    ).run(quiet=False)

    # CoreMark output (including its own "Total ticks" line) is printed to the
    # terminal via UART as the simulation runs.  We use the simulator's cycle
    # counter for our calculation — startup and teardown are negligible (<1%)
    # relative to 2000 benchmark iterations.
    cycles = result.stats["cycles"]
    ipc    = result.stats.get("ipc")

    coremark_per_mhz = ITERATIONS * 1e6 / cycles

    print()
    print("=" * 50)
    print(f"CoreMark/MHz        : {coremark_per_mhz:.3f}")
    print(f"Elapsed cycles      : {cycles:,}")
    if ipc is not None:
        print(f"IPC                 : {ipc:.3f}")
    print()
    print("Reference (bare-metal, no OS overhead):")
    print("  U74  in-order Linux  : ~2.63 CoreMark/MHz")
    print("  P550 OoO  bare-metal : ~3.0–3.5 expected")
    print("=" * 50)


if __name__ == "__main__":
    main()
