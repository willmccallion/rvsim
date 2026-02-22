"""
ARM Cortex-A72 machine config.
https://en.wikipedia.org/wiki/ARM_Cortex-A72

Microarchitecture (publicly documented):
- 3-wide fetch/decode/rename/dispatch/issue
- Out-of-order execution, 128-entry ROB
- 60-entry unified issue queue
- 12-entry store buffer, 16-entry load queue
- 2 load ports, 1 store port
- PRF: 128 integer + 128 FP physical registers
- Execution units:
    - 3x integer ALU (latency 1, includes shift/compare)
    - 1x integer multiplier (latency 3, pipelined)
    - 1x integer divider (latency ~20-39, non-pipelined)
    - 2x FP/NEON pipeline (modeled as FpAdd + FpMul/FpFma per pipe)
    - 1x FP div/sqrt (latency ~17-38, non-pipelined)
    - 1x branch unit (latency 1)
    - 2x load/store AGU (modeled as Mem units)
- 48KB L1-I (3-way), 32KB L1-D (2-way), 8 MSHRs on L1-D
- 1MB L2 (16-way unified), shared
- TAGE-like branch predictor, 4096-entry BTB, 16-entry RAS
"""

from rvsim import Backend, BranchPredictor, Cache, Config, Fu, Prefetcher


def cortex_a72_config():
    """Cortex-A72: 3-wide O3, 48KB I$, 32KB D$ (8 MSHRs), 1MB L2."""
    return Config(
        width=3,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=4,
            table_size=2048,
            loop_table_size=128,
            reset_interval=1000,
            history_lengths=[8, 20, 50, 110],
            tag_widths=[8, 8, 9, 9],
        ),
        backend=Backend.OutOfOrder(
            rob_size=128,
            issue_queue_size=60,
            store_buffer_size=12,
            load_queue_size=16,
            load_ports=2,
            store_ports=1,
            prf_gpr_size=128,
            prf_fpr_size=128,
            fu_config=Fu([
                Fu.IntAlu(count=3, latency=1),
                Fu.IntMul(count=1, latency=3),
                Fu.IntDiv(count=1, latency=28),
                Fu.FpAdd(count=2, latency=5),
                Fu.FpMul(count=2, latency=5),
                Fu.FpFma(count=2, latency=5),
                Fu.FpDivSqrt(count=1, latency=17),
                Fu.Branch(count=1, latency=1),
                Fu.Mem(count=2, latency=1),
            ]),
        ),
        btb_size=4096,
        ras_size=16,
        ram_size="256MB",
        tlb_size=48,
        l1i=Cache(
            size="48KB",
            line="64B",
            ways=3,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=2,
            latency=1,
            mshr_count=8,
            prefetcher=Prefetcher.Stride(degree=1, table_size=32),
        ),
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            latency=12,
            mshr_count=16,
        ),
    )


# Entry point for Simulator.config("scripts/cortex_a72/config.py")
config = cortex_a72_config
