"""
Design Space Exploration (Sweep) for RISC-V Architectures.

Sweeps two parameters using parallel execution:
1. L1 Data Cache Size: [16KB, 32KB, 64KB, 128KB]
2. Pipeline Width: [1, 2, 4, 8]

Utilizes `rvsim.Sweep` for parallel execution.

Usage:
  rvsim scripts/analysis/design_space.py [binary]
"""

import sys
import os
import argparse

# Add project root to path so we can import configs
_scripts = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_root = os.path.dirname(_scripts)
sys.path.insert(0, _root)

from rvsim import Sweep, Config, Cache, BranchPredictor

CACHE_SIZES = ["16KB", "32KB", "64KB", "128KB"]
WIDTHS = [1, 2, 4, 8]

def run_sweep(binary):
    print(f"Starting Parallel Design Space Exploration for {os.path.basename(binary)}...")
    print(f"Sweeping Widths: {WIDTHS}")
    print(f"Sweeping L1 D-Cache Sizes: {CACHE_SIZES}")
    print("-" * 60)
    
    # Generate configs
    configs = {}
    for width in WIDTHS:
        for size in CACHE_SIZES:
            label = f"w{width}_{size}"
            # Base config with TAGE predictor
            cfg = Config(
                width=width,
                branch_predictor=BranchPredictor.TAGE(),
                l1d=Cache(size, ways=4),
                l1i=Cache("32KB", ways=4),
                l2=Cache("1MB", ways=16),
                uart_quiet=True
            )
            configs[label] = cfg

    # Run sweep in parallel
    sweep = Sweep(binaries=[binary], configs=configs)
    sweep.run(parallel=True).compare(metrics=["ipc", "cycles", "dcache_misses"], col_header="Config")
    
if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Parallel Design Space Exploration")
    parser.add_argument("binary", nargs="?", default="software/bin/benchmarks/qsort.elf", help="Path to ELF binary")
    args = parser.parse_args()

    # Resolve binary path
    binary = args.binary
    if not os.path.exists(binary):
        # Try relative to project root
        candidate = os.path.join(_root, binary)
        if os.path.exists(candidate):
            binary = candidate
        else:
            print(f"Error: Binary {binary} not found.")
            sys.exit(1)

    run_sweep(binary)
