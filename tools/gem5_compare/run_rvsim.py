#!/usr/bin/env python3
"""
Run rvsim on the comparison programs under every variant and save the stats
to results/rvsim.json.

Usage:
    python tools/gem5_compare/run_rvsim.py [variant ...]

Runs every variant in variants.VARIANTS when none are named. The programs
come from tools/gem5_compare/programs/build.sh.
"""

import json
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))

from variants import (
    CLOCK_MHZ,
    VARIANTS,
    VECTOR_UNITS,
    VLEN,
    tage_history_lengths,
)

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Environment,
    Fu,
    MemoryController,
    Prefetcher,
)

RESULTS_DIR = Path(__file__).parent / "results"
PROGRAMS = ROOT / "tests/builds/compare-programs"


def branch_predictor(bp: dict):
    if bp["kind"] == "tournament":
        return BranchPredictor.Tournament(
            global_size_bits=bp["global_bits"],
            local_hist_bits=bp["local_hist_bits"],
            local_pred_bits=bp["local_pred_bits"],
        )
    if bp["kind"] == "tage":
        return BranchPredictor.TAGE(
            num_banks=bp["tables"],
            table_size=2 ** bp["log_table_size"],
            reset_interval=2 ** bp["log_u_reset"],
            history_lengths=tage_history_lengths(
                bp["min_hist"], bp["max_hist"], bp["tables"]
            ),
            tag_widths=bp["tag_widths"],
        )
    if bp["kind"] == "tage_sc_l":
        return BranchPredictor.ScLTage()
    raise ValueError(f"unknown predictor {bp['kind']}")


def cache(c: dict, prefetch_degree=None) -> Cache:
    return Cache(
        size=f"{c['size_kb']}KB",
        line="64B",
        ways=c["assoc"],
        latency=c["latency"],
        response_latency=c["response"],
        mshr_count=c["mshrs"],
        targets_per_mshr=c["tgts"],
        write_buffers=8,
        prefetcher=Prefetcher.Tagged(degree=prefetch_degree)
        if prefetch_degree
        else None,
    )


def functional_units() -> Fu:
    """gem5's O3 FU pool, unit for unit where rvsim has the same unit.

    Each vector unit gets VECTOR_UNITS copies at one cycle, gem5's
    `SIMD_Unit` pool, which takes a whole register per operation.
    """
    vector = [
        getattr(Fu, name)(count=VECTOR_UNITS, latency=1)
        for name in (
            "VecIntAlu",
            "VecIntMul",
            "VecIntDiv",
            "VecFpAlu",
            "VecFpFma",
            "VecFpDivSqrt",
            "VecPermute",
        )
    ]
    return Fu(
        [
            Fu.IntAlu(count=3, latency=1),
            Fu.IntMul(count=1, latency=3),  # IntMultDiv: IntMult
            Fu.IntDiv(count=1, latency=20),  # IntMultDiv: IntDiv, unpipelined
            Fu.FpAdd(count=2, latency=2),  # FP_ALU
            Fu.FpMul(count=2, latency=4),  # FP_MultDiv: FloatMult
            Fu.FpFma(count=2, latency=5),  # FP_MultDiv: FloatMultAcc
            Fu.FpDivSqrt(count=2, latency=12),  # FP_MultDiv: FloatDiv
            Fu.Branch(count=3, latency=1),  # branches issue to the IntALUs
            Fu.Mem(count=2, latency=1),  # one ReadPort plus one WritePort
            Fu.VecMem(count=1, latency=1),  # vector accesses use the same ports
            *vector,
        ]
    )


def config_for(variant: dict) -> Config:
    """The machine `gem5_single.py` builds for the same variant."""
    bus = variant["bus"]
    return Config(
        width=3,
        cpu_clock_mhz=CLOCK_MHZ,
        backend=Backend.OutOfOrder(
            rob_size=72,
            issue_queue_size=32,
            load_queue_size=24,
            store_buffer_size=16,
            prf_gpr_size=128,
            prf_fpr_size=96,
            load_ports=1,
            store_ports=1,
            fu_config=functional_units(),
        ),
        branch_predictor=branch_predictor(variant["bp"]),
        btb_size=4096,  # SimpleBTB: 4096 entries, direct-mapped
        btb_ways=1,
        ras_size=16,
        l1i=cache(variant["l1i"]),
        l1d=cache(variant["l1d"], variant["l1d_prefetch_degree"]),
        l2=cache(variant["l2"]),
        bus_width=bus["width_bytes"],
        bus_latency=bus["latency"],
        memory_controller=MemoryController.Simple(variant["memory"]["bandwidth_gib_s"]),
        vlen=VLEN,
        num_vec_lanes=VLEN // 8,
    )


def extract(stats: dict) -> dict:
    def get(key):
        return stats.get(key)

    return {
        "insts": get("instructions_retired"),
        "cycles": get("cycles"),
        "ipc": get("ipc"),
        "branches": get("core0.commit.op.branch"),
        "mispredicts": get("core0.bp.committed.mispredicts"),
        "loads": (get("core0.commit.op.load") or 0)
        + (get("core0.commit.vec.load") or 0),
        "stores": (get("core0.commit.op.store") or 0)
        + (get("core0.commit.vec.store") or 0),
        "l1i_misses": get("core0.cache.l1i.misses"),
        "l1d_accesses": (get("core0.cache.l1d.hits") or 0)
        + (get("core0.cache.l1d.misses") or 0),
        "l1d_misses": get("core0.cache.l1d.misses"),
        "l2_accesses": (get("core0.cache.l2.hits") or 0)
        + (get("core0.cache.l2.misses") or 0),
        "l2_misses": get("core0.cache.l2.misses"),
    }


def run_one(job: tuple[str, Path]) -> tuple[str, str, dict]:
    variant_name, binary = job
    config = config_for(VARIANTS[variant_name])
    result = Environment(binary=str(binary), config=config).run(quiet=True)
    return variant_name, binary.stem, extract(result.stats)


def main():
    variants = sys.argv[1:] or list(VARIANTS)
    binaries = sorted(PROGRAMS.glob("*.elf"))
    if not binaries:
        sys.exit(
            f"error: no programs in {PROGRAMS}; run tools/gem5_compare/programs/build.sh"
        )

    jobs = [(v, b) for v in variants for b in binaries]
    results: dict = {v: {} for v in variants}
    with ProcessPoolExecutor() as pool:
        for variant_name, program, stats in pool.map(run_one, jobs):
            results[variant_name][program] = stats
            print(
                f"  rvsim {variant_name:20} {program:22} IPC={stats['ipc']:.3f}",
                flush=True,
            )

    RESULTS_DIR.mkdir(exist_ok=True)
    out = RESULTS_DIR / "rvsim.json"
    # Variants not run this time keep their earlier results.
    saved = json.loads(out.read_text()) if out.exists() else {}
    saved.update(results)
    out.write_text(json.dumps(saved, indent=2, sort_keys=True))
    print(f"Saved: {out}")


if __name__ == "__main__":
    main()
