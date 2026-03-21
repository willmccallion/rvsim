"""
SiFive Performance P550 machine config.

Based on published microarchitecture analysis:
- Chips and Cheese: "Inside SiFive's P550 Microarchitecture" (Jan 2025)
- SiFive official specs: 13-stage, triple-issue, out-of-order, RV64GC
- Measured on Eswin EIC7700X SoC @ 1.4 GHz, 32KB+32KB L1, private L2, 4MB shared L3
- Published SPECInt2006: 8.65/GHz
- Observed IPC: approaching 3.0 on favorable workloads

Usage:
    from p550.config import p550_config
    config = p550_config()
"""

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    MemoryController,
    Prefetcher,
)


def p550_config(
    *,
    branch_predictor=None,
    ram_size_bytes="256MB",
    pipeline_width=3,
):
    """
    SiFive Performance P550 — 3-wide, 13-stage, out-of-order.

    Microarchitecture notes (from Chips and Cheese reverse engineering):
    - 3-wide fetch/decode/rename/retire
    - ROB ~72 entries (comparable to Core 2 / Goldmont Plus class)
    - Modest issue queue, ~32 entries estimated
    - Load queue ~24 entries, store buffer ~16 entries (described as "thin")
    - PRF sized with "plenty of capacity compared to ROB size"
    - 9.1 KiB branch history table with good pattern recognition
    - 32-entry BTB handles taken branches with zero bubbles
    - 32KB 8-way L1i, 32KB 8-way L1d, private L2 per core
    - 4 MB shared L3 on EIC7700X implementation
    - 13-stage pipeline → ~11-13 cycle mispredict penalty
    - No hardware misaligned access support (trap-based emulation)
    """
    if branch_predictor is None:
        # P550 has a 9.1 KiB BHT — predictor type unconfirmed.
        # Tournament sizing to match ~9 KB budget:
        #   global predictor:  2^13 entries × 2b = 2 KB
        #   choice (selector): 2^13 entries × 2b = 2 KB
        #   local hist table:  2^11 entries × 11b ≈ 2.75 KB
        #   local predictor:   2^11 entries × 2b  = 0.5 KB
        #   total ≈ 7.25 KB (closest we can get within the budget)
        branch_predictor = BranchPredictor.Tournament(
            global_size_bits=13,
            local_hist_bits=11,
            local_pred_bits=11,
        )

    return Config(
        width=pipeline_width,
        backend=Backend.OutOfOrder(
            rob_size=72,  # Modest, Core 2 / Goldmont Plus class
            issue_queue_size=32,  # "more scheduling capacity" than A75
            load_queue_size=24,  # "memory ordering queues can be a bit thin"
            store_buffer_size=16,  # Conservative estimate from "thin" description
            prf_gpr_size=128,  # "plenty of register file capacity compared to ROB"
            prf_fpr_size=96,  # Proportional to GPR, RV64GC needs FP regs
            load_ports=1,
            store_ports=1,
            fu_config=Fu(
                [
                    # "more flexible integer port setup" than A75
                    # P550 has 3 integer ALU ports based on execution width analysis
                    Fu.IntAlu(count=3, latency=1),
                    Fu.IntMul(count=1, latency=3),
                    Fu.IntDiv(count=1, latency=12),
                    # FP — single pipeline handles add/mul/fma; model as one
                    # of each since rvsim uses separate type classes.
                    # All share the same 5-cycle latency (P550 FP pipeline).
                    Fu.FpAdd(count=1, latency=5),
                    Fu.FpMul(count=1, latency=5),
                    Fu.FpFma(count=1, latency=5),
                    Fu.FpDivSqrt(count=1, latency=15),
                    Fu.Branch(count=1, latency=1),
                    Fu.Mem(count=1, latency=1),
                ]
            ),
        ),
        branch_predictor=branch_predictor,  # type: ignore[arg-type]
        btb_size=32,  # 32-entry BTB, zero-bubble taken branches
        ras_size=16,  # Modest RAS for low-power core
        initial_sp=0x8010_0000,
        ram_size=ram_size_bytes,
        # L1i: 32KB, 8-way, 64B lines — confirmed by SiFive specs
        l1i=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=1),
        ),
        # L1d: 32KB, 8-way, 64B lines — confirmed by SiFive specs
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=3,  # 3-cycle load-to-use (typical for this class)
            mshr_count=8,  # Non-blocking, modest MSHR count
            prefetcher=Prefetcher.Stride(degree=1, table_size=64),
        ),
        # Private L2 per core — size not publicly confirmed,
        # 256KB is consistent with area-optimized OoO cores
        l2=Cache(
            size="256KB",
            line="64B",
            ways=8,
            latency=10,
            mshr_count=16,
        ),
        # 4 MB shared L3 on EIC7700X — modeling single-core view
        l3=Cache(
            size="4MB",
            line="64B",
            ways=16,
            latency=30,
            mshr_count=32,
        ),
        # LPDDR5-6400 on the Premier P550 dev board
        memory_controller=MemoryController.DRAM(
            t_cas=14,
            t_ras=14,
            row_miss_latency=120,
        ),
    )


# Entry point for Simulator.config("scripts/p550/config.py")
config = p550_config
