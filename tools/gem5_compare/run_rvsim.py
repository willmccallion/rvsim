#!/usr/bin/env python3
"""
Run rvsim on the comparison programs under every variant and save the stats
to results/rvsim.json.

Usage:
    python tools/gem5_compare/run_rvsim.py [variant ...]

Runs every variant in variants.VARIANTS when none are named. The programs
come from tools/gem5_compare/programs/build.sh.
"""

import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

ROOT = Path(__file__).parent.parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))

import results
from variants import (
    CLOCK_MHZ,
    VARIANTS,
    VECTOR_UNITS,
    VLEN,
    Variant,
    tage_history_lengths,
)
from variants import BranchPredictor as BranchPredictorSpec
from variants import Cache as CacheSpec

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    MemoryController,
    Prefetcher,
    Simulator,
)
from rvsim._core import StatsTree

# What gem5's SE loader leaves a process, which the programs' start-up
# relies on: a stack (software/libc/user.ld's _stack_top) and the FPU and
# vector unit switched on.
STACK_TOP = 0x8100_0000
SP = 2
MSTATUS_FS_VS_INITIAL = 0x2200


def branch_predictor(
    bp: BranchPredictorSpec,
) -> BranchPredictor.Tournament | BranchPredictor.Tage | BranchPredictor.ScLTage:
    if bp["kind"] == "tournament":
        return BranchPredictor.Tournament(
            global_size_bits=bp["global_bits"],
            local_hist_bits=bp["local_hist_bits"],
            local_pred_bits=bp["local_pred_bits"],
        )
    if bp["kind"] == "tage":
        return BranchPredictor.Tage(
            num_banks=bp["tables"],
            table_size=2 ** bp["log_table_size"],
            reset_interval=2 ** bp["log_u_reset"],
            history_lengths=tage_history_lengths(
                bp["min_hist"], bp["max_hist"], bp["tables"]
            ),
            tag_widths=bp["tag_widths"],
        )
    return BranchPredictor.ScLTage()


def l1d_prefetcher(variant: Variant) -> Prefetcher.Stride | Prefetcher.Tagged | None:
    stride = variant["l1d_stride_prefetch"]
    if stride:
        return Prefetcher.Stride(
            degree=stride["degree"], table_size=stride["table_size"]
        )
    degree = variant["l1d_prefetch_degree"]
    return Prefetcher.Tagged(degree=degree) if degree else None


def cache(
    c: CacheSpec, prefetcher: Prefetcher.Stride | Prefetcher.Tagged | None = None
) -> Cache:
    return Cache(
        size=f"{c['size_kb']}KB",
        line="64B",
        ways=c["assoc"],
        latency=c["latency"],
        response_latency=c["response"],
        mshr_count=c["mshrs"],
        targets_per_mshr=c["tgts"],
        write_buffers=8,
        prefetcher=prefetcher,
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


def config_for(variant: Variant) -> Config:
    """The machine `gem5_single.py` builds for the same variant."""
    bus = variant["bus"]
    return Config(
        width=3,
        cpu_clock_mhz=CLOCK_MHZ,
        # gem5's LSQ gives a forwarded load one cycle: its writeback event
        # lands in the cycle after the load executes.
        store_forward_latency=1,
        # gem5's O3 hands a misprediction from IEW to commit and from commit
        # to fetch, a cycle each (`iewToCommitDelay`, `commitToFetchDelay`).
        redirect_latency=2,
        # gem5's O3 takes a trap `trapLatency` cycles after commit finds it.
        trap_latency=13,
        backend=Backend.OutOfOrder(
            rob_size=72,
            issue_queue_size=32,
            load_queue_size=24,
            store_buffer_size=16,
            prf_gpr_size=128,
            prf_fpr_size=96,
            load_ports=1,
            store_ports=1,
            prf_vpr_size=64,
            fu_config=functional_units(),
            # gem5's O3 keeps no rename checkpoints: it undoes its history
            # buffer `squashWidth` entries a cycle.
            checkpoint_count=0,
            squash_width=8,
        ),
        branch_predictor=branch_predictor(variant["bp"]),
        btb_size=4096,  # SimpleBTB: 4096 entries, direct-mapped
        btb_ways=1,
        ras_size=16,
        l1i=cache(variant["l1i"]),
        l1d=cache(variant["l1d"], l1d_prefetcher(variant)),
        l2=cache(variant["l2"]),
        l3=None,
        wcb_entries=0,
        load_prefetcher=None,
        store_prefetcher=None,
        l2_tlb_size=0,
        l2_tlb_ways=4,
        l2_tlb_latency=4,
        # gem5's RISC-V ITB and DTB each have a walker (RiscvTLB.py).
        walker_sharing="PerSide",
        bus_width=bus["width_bytes"],
        bus_latency=bus["latency"],
        memory_controller=MemoryController.Simple(
            latency=variant["memory"]["latency"],
            bandwidth_gib_s=variant["memory"]["bandwidth_gib_s"],
        ),
        vlen=VLEN,
        num_vec_lanes=VLEN // 8,
    )


def extract(stats: StatsTree) -> results.ProgramStats:
    """The stats compare.py reads, from one run's stats tree."""

    def total(*paths: str) -> float:
        return sum(stats.get(path) or 0.0 for path in paths)

    return {
        "insts": stats.instructions_retired,
        "cycles": stats.cycles,
        "ipc": stats.ipc,
        "branches": stats.get("core0.commit.op.branch"),
        "mispredicts": stats.get("core0.bp.committed.mispredicts"),
        "loads": total("core0.commit.op.load", "core0.commit.vec.load"),
        "stores": total("core0.commit.op.store", "core0.commit.vec.store"),
        "l1i_misses": stats.get("core0.cache.l1i.misses"),
        "l1d_accesses": total("core0.cache.l1d.hits", "core0.cache.l1d.misses"),
        "l1d_misses": stats.get("core0.cache.l1d.misses"),
        "l2_accesses": total("core0.cache.l2.hits", "core0.cache.l2.misses"),
        "l2_misses": stats.get("core0.cache.l2.misses"),
    }


def run_one(job: tuple[str, Path]) -> tuple[str, str, results.ProgramStats]:
    variant_name, binary = job
    sim = Simulator(config_for(VARIANTS[variant_name]), binary=str(binary), mode="bare")
    sim.regs[SP] = STACK_TOP
    sim.csrs["mstatus"] |= MSTATUS_FS_VS_INITIAL
    exit_code = sim.run()
    if exit_code != 0:
        raise RuntimeError(f"{binary.name} on {variant_name} exited with {exit_code}")
    return variant_name, binary.stem, extract(sim.stats)


def main() -> None:
    variants = sys.argv[1:] or list(VARIANTS)
    jobs = [(v, b) for v in variants for b in results.program_binaries()]
    runs: results.Results = {v: {} for v in variants}
    with ProcessPoolExecutor() as pool:
        for variant_name, program, stats in pool.map(run_one, jobs):
            runs[variant_name][program] = stats
            print(
                f"  rvsim {variant_name:20} {program:22} IPC={stats['ipc']:.3f}",
                flush=True,
            )
    print(f"Saved: {results.save('rvsim', runs)}")


if __name__ == "__main__":
    main()
