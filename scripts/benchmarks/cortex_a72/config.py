"""
ARM Cortex-A72-style machine config.
https://en.wikipedia.org/wiki/ARM_Cortex-A72

- 3-wide fetch/decode/rename/dispatch
- Out-of-Order Execution
- 48KB L1-I (3-way)
- 32KB L1-D (2-way)
- 1MB L2 (16-way shared)
- Branch Prediction: Sophisticated (Approx. TAGE-like)
"""

from rvsim import Backend, BranchPredictor, Cache, Config, Prefetcher


def cortex_a72_config():
    """Cortex-A72: 3-wide O3, 48KB I$, 32KB D$, 1MB L2."""
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
        backend=Backend.OutOfOrder(),
        btb_size=4096,
        ras_size=16,
        ram_size="256MB",
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
            prefetcher=Prefetcher.Stride(degree=1, table_size=32),
        ),
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            latency=12,
        ),
    )


# Entry point for Simulator.config("scripts/cortex_a72/config.py")
config = cortex_a72_config
