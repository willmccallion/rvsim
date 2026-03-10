#!/usr/bin/env python3
"""
Run rvsim on a set of binaries and save stats to results/rvsim.json.

Usage:
    python scripts/comparison/run_rvsim.py [binary.elf ...]

Defaults to the standard benchmark set if no binaries are given.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent
sys.path.insert(0, str(ROOT))

from rvsim import Environment, Config, Backend, BranchPredictor, Cache, Fu

RESULTS_DIR = Path(__file__).parent / "results"

BENCH = ROOT / "software/bin/benchmarks"

DEFAULT_BINARIES = [
    BENCH / "mix_matrix_mul.elf",
    BENCH / "cache_linear_read.elf",
    BENCH / "cache_strided_read.elf",
    BENCH / "cache_thrash_assoc.elf",
    BENCH / "cache_write_heavy.elf",
    BENCH / "bp_random.elf",
    BENCH / "bp_pattern_alt.elf",
    BENCH / "bp_always_taken.elf",
    BENCH / "bp_never_taken.elf",
    BENCH / "alu_int_mul.elf",
    BENCH / "alu_int_div.elf",
    BENCH / "alu_fp_add.elf",
    BENCH / "pipe_load_use.elf",
    BENCH / "pipe_raw_hazard.elf",
    BENCH / "mem_rand_walk.elf",
]


def p550_config() -> Config:
    return Config(
        width=3,
        backend=Backend.OutOfOrder(
            rob_size=72,
            issue_queue_size=32,
            load_queue_size=24,
            store_buffer_size=16,
            prf_gpr_size=128,
            prf_fpr_size=96,
            load_ports=1,
            store_ports=1,
            fu_config=Fu([
                Fu.IntAlu(count=3, latency=1),
                Fu.IntMul(count=1, latency=3),
                Fu.IntDiv(count=1, latency=12),
                Fu.FpAdd(count=1, latency=5),
                Fu.FpMul(count=1, latency=5),
                Fu.FpFma(count=1, latency=5),
                Fu.FpDivSqrt(count=1, latency=15),
                Fu.Branch(count=1, latency=1),
                Fu.Mem(count=1, latency=1),
            ]),
        ),
        branch_predictor=BranchPredictor.Tournament(
            global_size_bits=13,
            local_hist_bits=11,
            local_pred_bits=11,
        ),
        btb_size=32,
        ras_size=16,
        l1i=Cache(size="32KB", line="64B", ways=8, latency=1),
        l1d=Cache(size="32KB", line="64B", ways=8, latency=3, mshr_count=8),
        l2=Cache(size="256KB", line="64B", ways=8, latency=10),
    )


def run(binaries: list[Path]) -> dict:
    config = p550_config()
    results = {}
    for binary in binaries:
        name = binary.stem
        print(f"  rvsim: {name}...", end=" ", flush=True)
        result = Environment(binary=str(binary), config=config).run(quiet=True)
        s = result.stats
        results[name] = {
            "ipc":     s.get("ipc"),
            "cycles":  s.get("sim_cycles"),
            "insts":   s.get("sim_insts"),
            "bp_acc":  s.get("bp_committed_accuracy"),
            "l1d_miss_rate": s.get("l1d_miss_rate"),
            "l2_miss_rate":  s.get("l2_miss_rate"),
        }
        print(f"IPC={results[name]['ipc']:.4f}")
    return results


def main():
    binaries = [Path(a) for a in sys.argv[1:]] if len(sys.argv) > 1 else DEFAULT_BINARIES
    missing = [b for b in binaries if not b.exists()]
    if missing:
        for m in missing:
            print(f"error: binary not found: {m}", file=sys.stderr)
        sys.exit(1)

    print("Running rvsim...")
    results = run(binaries)

    RESULTS_DIR.mkdir(exist_ok=True)
    out = RESULTS_DIR / "rvsim.json"
    out.write_text(json.dumps(results, indent=2))
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
