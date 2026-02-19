"""
M1-style machine config.

Usage:
    from m1.config import m1_config
    config = m1_config(branch_predictor=BranchPredictor.TAGE())
"""

import sys
import os

_repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _repo)

from rvsim import (
    Config,
    Cache,
    BranchPredictor,
    ReplacementPolicy,
    Prefetcher,
)


def m1_config(
    *,
    branch_predictor=None,
    ram_size_bytes=0x1000_0000,
    pipeline_width=4,
):
    """M1-style: 4-wide, 128KB L1-I/D, 4MB L2."""
    if branch_predictor is None:
        branch_predictor = BranchPredictor.TAGE(
            num_banks=4,
            table_size=4096,
            loop_table_size=512,
            reset_interval=2000,
            history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10],
        )

    return Config(
        width=pipeline_width,
        branch_predictor=branch_predictor,
        btb_size=8192,
        ras_size=64,
        start_pc=0x8000_0000,
        direct_mode=True,
        initial_sp=0x8010_0000,
        ram_size=ram_size_bytes,
        l1i=Cache(
            size="128KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        l1d=Cache(
            size="128KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.Stride(degree=2, table_size=128),
        ),
        l2=Cache(
            size="4MB",
            line="64B",
            ways=16,
            latency=12,
        ),
    )


# Entry point for Simulator.config("scripts/m1/config.py")
config = m1_config
