"""
Top-Down Microarchitecture Analysis (TMA) for RISC-V via rvsim.

Categorizes execution cycles into four main buckets:
1. Retiring (Useful work)
2. Bad Speculation (Wasted work due to branch mispredicts)
3. Frontend Bound (Starvation due to I-cache miss / fetch bandwidth)
4. Backend Bound (Stalls due to dependencies or memory latency)

Usage:
  rvsim scripts/analysis/top_down.py [binary]
  rvsim scripts/analysis/top_down.py --config m1
"""

import argparse
import importlib.util
import os
import sys
from pathlib import Path

from rvsim import Simulator, Stats

_ROOT = Path(__file__).resolve().parent.parent.parent
_BENCH = _ROOT / "scripts" / "benchmarks"


def _load_config_module(name: str):
    """Load a benchmark config module by directory name."""
    path = _BENCH / name / "config.py"
    spec = importlib.util.spec_from_file_location(f"{name}.config", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


p550_config = _load_config_module("p550").p550_config
m1_config = _load_config_module("m1").m1_config
cortex_a72_config = _load_config_module("cortex_a72").cortex_a72_config

CONFIGS = {
    "p550": p550_config,
    "m1": m1_config,
    "a72": cortex_a72_config,
}

def analyze_top_down(stats, width):
    """
    Compute Top-Down metrics using Slot-based accounting.
    
    Total Slots = Cycles * Pipeline Width
    
    Categories:
    1. Retiring: Actual instructions retired.
    2. Bad Speculation: Slots wasted due to branch misprediction recovery.
       - Approximated as 'stalls_control' cycles * width.
    3. Backend Bound: Slots wasted due to backend resource/data stalls.
       - Approximated as 'stalls_mem' + 'stalls_data' cycles * width.
    4. Frontend Bound: The remaining empty slots.
       - Represents fetch bubbles, I-cache misses, or just vertical waste
         (retiring 1 instr/cycle on a 3-wide machine due to dependencies).
    """
    cycles = stats.get("cycles", 1)
    retired = stats.get("instructions_retired", 0)
    
    # Stall cycles (assumed to be full-pipeline stalls where 0 insts retire)
    s_mem = stats.get("stalls_mem", 0)
    s_data = stats.get("stalls_data", 0)
    s_ctrl = stats.get("stalls_control", 0)
    
    total_slots = cycles * width
    if total_slots == 0: return {}

    # 1. Retiring
    # Exact count of useful slots filled
    slots_retiring = retired
    
    # 2. Bad Speculation
    # 'stalls_control' are cycles lost to flushing/recovery. 
    # All slots in these cycles are wasted.
    slots_bad_spec = s_ctrl * width
    
    # 3. Backend Bound
    # Cycles where backend structures (ROB, LSQ, RS) were full or waiting.
    slots_backend = (s_mem + s_data) * width
    
    # 4. Frontend Bound (and vertical waste)
    # The remainder. This catches:
    # - I-Cache misses (fetch bubbles)
    # - Dependency chains (only retiring 1 uOp instead of 'width')
    # - Fetch bandwidth limitations
    slots_frontend = total_slots - slots_retiring - slots_bad_spec - slots_backend
    slots_frontend = max(0, slots_frontend)

    return {
        "Retiring": (slots_retiring / total_slots) * 100.0,
        "Bad Speculation": (slots_bad_spec / total_slots) * 100.0,
        "Backend Bound": (slots_backend / total_slots) * 100.0,
        "Frontend Bound": (slots_frontend / total_slots) * 100.0,
        "IPC": stats.get("ipc", 0.0),
    }

def main():
    parser = argparse.ArgumentParser(description="Top-Down Performance Analysis")
    parser.add_argument("binary", nargs="?", default="software/bin/benchmarks/qsort.elf", help="Path to ELF binary")
    parser.add_argument("--config", choices=CONFIGS.keys(), default="a72", help="CPU Configuration")
    args = parser.parse_args()

    # Resolve binary path
    binary = args.binary
    if not os.path.exists(binary):
        candidate = _ROOT / binary
        if candidate.exists():
            binary = str(candidate)
        else:
            print(f"Error: Binary {binary} not found.")
            return

    print(f"Top-Down Analysis for {os.path.basename(binary)} on {args.config.upper()}...")
    
    # Build & Run
    cfg_func = CONFIGS[args.config]
    config = cfg_func().replace(uart_quiet=True)
    sim = Simulator().config(config).binary(binary)
    cpu = sim.build()
    cpu.run()
    
    stats = Stats(cpu.stats)
    metrics = analyze_top_down(stats, config.width)
    
    # Display
    print("\n" + "="*40)
    print(f" {args.config.upper()} Core Performance Summary")
    print("="*40)
    print(f" IPC: {metrics['IPC']:.2f} (Max possible: {config.width})")
    print("-" * 40)
    print(" Category breakdown:")
    print(f"  \033[32mRetiring:\033[0m         {metrics['Retiring']:5.1f}%")
    print(f"  \033[31mBad Speculation:\033[0m  {metrics['Bad Speculation']:5.1f}%")
    print(f"  \033[33mBackend Bound:\033[0m    {metrics['Backend Bound']:5.1f}%")
    print(f"  \033[36mFrontend Bound:\033[0m   {metrics['Frontend Bound']:5.1f}%")
    print("="*40)
    
    # Suggestions based on bottleneck
    bottleneck = max(metrics, key=lambda k: metrics[k] if k != "IPC" else -1)
    print(f"\nMain Bottleneck: {bottleneck}")
    if bottleneck == "Bad Speculation":
        print(" -> Suggestion: Improve Branch Predictor (larger history/tables).")
    elif bottleneck == "Backend Bound":
        print(" -> Suggestion: Increase Cache size, ROB size, or memory bandwidth.")
    elif bottleneck == "Frontend Bound":
        print(" -> Suggestion: Improve I-Cache or Fetch width.")
    
if __name__ == "__main__":
    main()
