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
    """The machine `gem5_single.py` builds, with gem5's defaults filled in.

    Each value matches the gem5 parameter noted beside it; the README lists
    the structural differences no rvsim setting can express.
    """
    return Config(
        width=3,
        cpu_clock_mhz=1400,  # SimpleBoard clk_freq
        backend=Backend.OutOfOrder(
            rob_size=72,
            issue_queue_size=32,
            load_queue_size=24,
            store_buffer_size=16,
            prf_gpr_size=128,
            prf_fpr_size=96,
            load_ports=1,  # ReadPort(count=1)
            store_ports=1,  # WritePort(count=1)
            fu_config=Fu([
                Fu.IntAlu(count=3, latency=1),
                Fu.IntMul(count=1, latency=3),  # IntMultDiv: IntMult
                Fu.IntDiv(count=1, latency=20),  # IntMultDiv: IntDiv, unpipelined
                Fu.FpAdd(count=2, latency=2),  # FP_ALU
                Fu.FpMul(count=2, latency=4),  # FP_MultDiv: FloatMult
                Fu.FpFma(count=2, latency=5),  # FP_MultDiv: FloatMultAcc
                Fu.FpDivSqrt(count=2, latency=12),  # FP_MultDiv: FloatDiv
                Fu.Branch(count=3, latency=1),  # branches issue to the IntALUs
                Fu.Mem(count=2, latency=1),  # one ReadPort plus one WritePort
            ]),
        ),
        branch_predictor=BranchPredictor.Tournament(
            global_size_bits=13,
            local_hist_bits=11,
            local_pred_bits=11,
        ),
        btb_size=4096,  # SimpleBTB: 4096 entries, direct-mapped
        btb_ways=1,
        ras_size=16,
        l1i=Cache(size="32KB", line="64B", ways=8, latency=1, mshr_count=16, write_buffers=8),
        l1d=Cache(size="32KB", line="64B", ways=8, latency=1, mshr_count=16, write_buffers=8),
        l2=Cache(
            size="256KB",
            line="64B",
            ways=16,
            latency=10,
            mshr_count=20,
            write_buffers=8,
            targets_per_mshr=12,
        ),
    )


def run(binaries: list[Path]) -> dict:
    config = p550_config()
    results = {}
    for binary in binaries:
        name = binary.stem
        print(f"  rvsim: {name}...", end=" ", flush=True)
        result = Environment(binary=str(binary), config=config).run(quiet=True)
        s = result.stats
        accuracy = s.get("core0.bp.committed.accuracy")
        results[name] = {
            "ipc": s.get("ipc"),
            "cycles": s.get("cycles"),
            "insts": s.get("instructions_retired"),
            "mispreds": s.get("core0.bp.committed.mispredicts"),
            "bp_acc": None if accuracy is None else accuracy * 100,
            "l1d_miss_rate": s.get("core0.cache.l1d.miss_rate"),
            "l2_miss_rate": s.get("core0.cache.l2.miss_rate"),
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
