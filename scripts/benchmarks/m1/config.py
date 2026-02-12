"""
M1-style machine config. Edit this file to change cache sizes, width, BP, etc.

Usage:
    from m1.config import m1_config
    config = m1_config(branch_predictor="TAGE")
"""
import sys
import os

_repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, _repo)

from riscv_sim import SimConfig
from riscv_sim.config import (
    TageConfig,
    PerceptronConfig,
    TournamentConfig,
)


def m1_config(
    *,
    branch_predictor="TAGE",
    ram_size_bytes=0x1000_0000,
    pipeline_width=4,
):
    """
    M1-style: 4-wide, 128KB L1-I/D, 4MB L2.
    """
    c = SimConfig.default()
    c.general.trace_instructions = False
    c.general.start_pc = 0x8000_0000
    c.general.direct_mode = True
    c.general.initial_sp = 0x8010_0000
    c.memory.ram_size = ram_size_bytes
    c.memory.controller = "Simple"
    c.memory.t_cas = 14
    c.memory.t_ras = 14
    c.memory.t_pre = 14
    c.memory.row_miss_latency = 120
    c.memory.tlb_size = 32

    # L1-I: 128KB
    c.cache.l1_i.enabled = True
    c.cache.l1_i.size_bytes = 128 * 1024
    c.cache.l1_i.line_bytes = 64
    c.cache.l1_i.ways = 8
    c.cache.l1_i.policy = "LRU"
    c.cache.l1_i.latency = 1
    c.cache.l1_i.prefetcher = "NextLine"
    c.cache.l1_i.prefetch_degree = 2

    # L1-D: 128KB
    c.cache.l1_d.enabled = True
    c.cache.l1_d.size_bytes = 128 * 1024
    c.cache.l1_d.line_bytes = 64
    c.cache.l1_d.ways = 8
    c.cache.l1_d.policy = "LRU"
    c.cache.l1_d.latency = 1
    c.cache.l1_d.prefetcher = "Stride"
    c.cache.l1_d.prefetch_table_size = 128
    c.cache.l1_d.prefetch_degree = 2

    # L2: 4MB
    c.cache.l2.enabled = True
    c.cache.l2.size_bytes = 4 * 1024 * 1024
    c.cache.l2.line_bytes = 64
    c.cache.l2.ways = 16
    c.cache.l2.policy = "LRU"
    c.cache.l2.latency = 12
    c.cache.l2.prefetcher = "None"

    c.cache.l3.enabled = False
    c.cache.l3.size_bytes = 0
    c.cache.l3.line_bytes = 0
    c.cache.l3.ways = 0

    c.pipeline.width = pipeline_width
    c.pipeline.branch_predictor = branch_predictor
    c.pipeline.btb_size = 8192
    c.pipeline.ras_size = 64
    c.pipeline.tage = TageConfig(
        num_banks=4,
        table_size=4096,
        loop_table_size=512,
        reset_interval=2000,
        history_lengths=[5, 15, 44, 130],
        tag_widths=[9, 9, 10, 10],
    )
    c.pipeline.perceptron = PerceptronConfig(history_length=32, table_bits=10)
    c.pipeline.tournament = TournamentConfig(
        global_size_bits=12,
        local_hist_bits=10,
        local_pred_bits=10,
    )
    return c


# Entry point for Simulator.config("scripts/m1/config.py")
config = m1_config
