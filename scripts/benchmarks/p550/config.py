"""
P550-style machine config.

Usage:
    from p550.config import p550_config
    config = p550_config(branch_predictor=BranchPredictor.TAGE())
"""

import os
import sys

_repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _repo)

from rvsim import BranchPredictor, Cache, Config, Prefetcher


def p550_config(
    *,
    branch_predictor=None,
    ram_size_bytes="256MB",
    pipeline_width=3,
):
    """P550-style: 3-wide, 32KB L1-I/D, 256KB L2, TAGE."""
    if branch_predictor is None:
        branch_predictor = BranchPredictor.TAGE(
            num_banks=4,
            table_size=2048,
            loop_table_size=256,
            reset_interval=2000,
            history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10],
        )

    return Config(
        width=pipeline_width,
        branch_predictor=branch_predictor,
        btb_size=4096,
        ras_size=32,
        initial_sp=0x8010_0000,
        ram_size=ram_size_bytes,
        l1i=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=1),
        ),
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.Stride(degree=1, table_size=64),
        ),
        l2=Cache(
            size="256KB",
            line="64B",
            ways=8,
            latency=10,
        ),
    )


# Entry point for Simulator.config("scripts/p550/config.py")
config = p550_config
