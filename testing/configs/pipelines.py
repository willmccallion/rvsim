"""Shared pipeline-config matrix used by every test runner under testing/.

A test runner imports `PIPELINES` from here and iterates over (label, cfg)
tuples. Each Config exercises a different rvsim configuration: pipeline width,
backend, branch predictor, ROB sizing, FU pool, cache hierarchy, prefetcher,
replacement policy, memory controller, mem-dep predictor, TLB, and the four
reference machine configs (cortex-a72, p550, m1, linux).

If you add or remove a Config here, every runner picks it up automatically —
this is the single source of truth.
"""

import os
import sys

# The reference machine configs live under scripts/benchmarks/<name>/config.py
# and scripts/setup/boot_linux.py. Pull them in via path manipulation so the
# runners don't have to.
_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(_REPO_ROOT, "scripts", "benchmarks"))
sys.path.insert(0, os.path.join(_REPO_ROOT, "scripts"))

from rvsim import (  # noqa: E402
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    MemDepPredictor,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)
from cortex_a72.config import cortex_a72_config  # noqa: E402
from m1.config import m1_config  # noqa: E402
from p550.config import p550_config  # noqa: E402
from setup.boot_linux import config as linux_config  # noqa: E402

# fmt: off
PIPELINES = [
    # ── In-order widths ───────────────────────────────────────────────────────
    ("inorder w1",          Config(width=1, backend=Backend.InOrder())),
    ("inorder w2",          Config(width=2, backend=Backend.InOrder())),
    ("inorder w3",          Config(width=3, backend=Backend.InOrder())),
    ("inorder w4",          Config(width=4, backend=Backend.InOrder())),

    # ── O3 widths ─────────────────────────────────────────────────────────────
    ("o3 w1",               Config(width=1, backend=Backend.OutOfOrder())),
    ("o3 w2",               Config(width=2, backend=Backend.OutOfOrder())),
    ("o3 w3",               Config(width=3, backend=Backend.OutOfOrder())),
    ("o3 w4",               Config(width=4, backend=Backend.OutOfOrder())),
    ("o3 w8",               Config(width=8, backend=Backend.OutOfOrder())),

    # ── ROB / IQ / buffer sizing ──────────────────────────────────────────────
    # Smallest viable machine: structures fill immediately, constant flush/re-fill.
    ("o3 w4 small-rob",     Config(width=4, backend=Backend.OutOfOrder(
        rob_size=16, issue_queue_size=8,
        store_buffer_size=4, load_queue_size=8,
        prf_gpr_size=64, prf_fpr_size=64,
    ))),
    # Oversized: structures never fill, exposes wakeup correctness at distance.
    ("o3 w4 large-rob",     Config(width=4, backend=Backend.OutOfOrder(
        rob_size=256, issue_queue_size=128,
        store_buffer_size=64, load_queue_size=64,
        prf_gpr_size=384, prf_fpr_size=256,
    ))),
    # PRF exactly at the architectural minimum: rob_size + 32 each.
    ("o3 w4 tight-prf",     Config(width=4, backend=Backend.OutOfOrder(
        rob_size=64, prf_gpr_size=96, prf_fpr_size=96,
    ))),
    # Wide OOO window on a single-issue pipe: does out-of-order help at w=1?
    ("o3 w1 large-rob",     Config(width=1, backend=Backend.OutOfOrder(
        rob_size=128, issue_queue_size=64,
    ))),

    # ── Branch predictors ─────────────────────────────────────────────────────
    ("o3 w4 static",        Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.Static())),
    ("o3 w4 gshare",        Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.GShare())),
    ("o3 w4 tournament",    Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.Tournament())),
    ("o3 w4 perceptron",    Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.Perceptron())),
    # TAGE with more banks and larger tables than the default.
    ("o3 w4 tage-wide",     Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.TAGE(
                                       num_banks=6, table_size=4096,
                                       loop_table_size=512, reset_interval=2000,
                                       history_lengths=[5, 15, 44, 130, 320, 800],
                                       tag_widths=[9, 9, 10, 10, 11, 11],
                                   ))),
    # SC-L-TAGE with default parameters.
    ("o3 w4 sc-l-tage",     Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.ScLTage())),
    # SC-L-TAGE with larger tables (matches linux config scale).
    ("o3 w4 sc-l-tage-wide", Config(width=4, backend=Backend.OutOfOrder(),
                                   branch_predictor=BranchPredictor.ScLTage(
                                       num_banks=8, table_size=4096,
                                       loop_table_size=512, reset_interval=500_000,
                                       history_lengths=[5, 11, 22, 44, 89, 178, 356, 712],
                                       tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
                                       sc_num_tables=6, sc_table_size=1024,
                                       ittage_num_banks=8, ittage_table_size=512,
                                   ))),

    # ── BTB / RAS sizing ──────────────────────────────────────────────────────
    # Tiny BTB: most targets alias, stresses cold-start and aliasing penalty.
    ("o3 w4 small-btb",     Config(width=4, backend=Backend.OutOfOrder(),
                                   btb_size=64, ras_size=4)),
    ("o3 w4 large-btb",     Config(width=4, backend=Backend.OutOfOrder(),
                                   btb_size=16384, ras_size=64)),

    # ── FU pool: one unit of each type ────────────────────────────────────────
    # Every FU class becomes a bottleneck; hits structural stall for all types.
    # Vector FUs are required: when fu_config is overridden, the rvsim FU pool
    # creates ONLY the units listed here, so omitting Vec* would deadlock any
    # vector op. Each variant scales scalar and vector pools together.
    ("o3 w4 fu-minimal",    Config(width=4, backend=Backend.OutOfOrder(fu_config=Fu([
        Fu.IntAlu(count=1, latency=1),
        Fu.IntMul(count=1, latency=3),
        Fu.IntDiv(count=1, latency=35),
        Fu.FpAdd(count=1, latency=4),
        Fu.FpMul(count=1, latency=5),
        Fu.FpFma(count=1, latency=5),
        Fu.FpDivSqrt(count=1, latency=21),
        Fu.Branch(count=1, latency=1),
        Fu.Mem(count=1, latency=1),
        Fu.VecIntAlu(count=1, latency=1),
        Fu.VecIntMul(count=1, latency=3),
        Fu.VecIntDiv(count=1, latency=35),
        Fu.VecFpAlu(count=1, latency=4),
        Fu.VecFpFma(count=1, latency=5),
        Fu.VecFpDivSqrt(count=1, latency=21),
        Fu.VecMem(count=1, latency=1),
        Fu.VecPermute(count=1, latency=1),
    ])))),
    # Two of each, moderate latencies: intermediate structural pressure.
    ("o3 w4 fu-mid",        Config(width=4, backend=Backend.OutOfOrder(fu_config=Fu([
        Fu.IntAlu(count=2, latency=1),
        Fu.IntMul(count=2, latency=4),
        Fu.IntDiv(count=1, latency=35),
        Fu.FpAdd(count=2, latency=6),
        Fu.FpMul(count=2, latency=7),
        Fu.FpFma(count=2, latency=7),
        Fu.FpDivSqrt(count=1, latency=30),
        Fu.Branch(count=2, latency=1),
        Fu.Mem(count=2, latency=1),
        Fu.VecIntAlu(count=2, latency=1),
        Fu.VecIntMul(count=2, latency=4),
        Fu.VecIntDiv(count=1, latency=35),
        Fu.VecFpAlu(count=2, latency=6),
        Fu.VecFpFma(count=2, latency=7),
        Fu.VecFpDivSqrt(count=1, latency=30),
        Fu.VecMem(count=2, latency=1),
        Fu.VecPermute(count=2, latency=1),
    ])))),
    # Many units, long latencies: deep in-flight chains, stresses wakeup logic.
    ("o3 w4 fu-wide",       Config(width=4, backend=Backend.OutOfOrder(fu_config=Fu([
        Fu.IntAlu(count=8, latency=1),
        Fu.IntMul(count=4, latency=6),
        Fu.IntDiv(count=2, latency=35),
        Fu.FpAdd(count=4, latency=8),
        Fu.FpMul(count=4, latency=10),
        Fu.FpFma(count=4, latency=10),
        Fu.FpDivSqrt(count=2, latency=40),
        Fu.Branch(count=4, latency=1),
        Fu.Mem(count=4, latency=1),
        Fu.VecIntAlu(count=8, latency=1),
        Fu.VecIntMul(count=4, latency=6),
        Fu.VecIntDiv(count=2, latency=35),
        Fu.VecFpAlu(count=4, latency=8),
        Fu.VecFpFma(count=4, latency=10),
        Fu.VecFpDivSqrt(count=2, latency=40),
        Fu.VecMem(count=4, latency=1),
        Fu.VecPermute(count=4, latency=1),
    ])))),
    # High latencies across all units: maximises data-hazard stall counts.
    ("o3 w4 fu-slow",       Config(width=4, backend=Backend.OutOfOrder(fu_config=Fu([
        Fu.IntAlu(count=4, latency=4),
        Fu.IntMul(count=1, latency=10),
        Fu.IntDiv(count=1, latency=60),
        Fu.FpAdd(count=2, latency=10),
        Fu.FpMul(count=2, latency=12),
        Fu.FpFma(count=2, latency=12),
        Fu.FpDivSqrt(count=1, latency=50),
        Fu.Branch(count=2, latency=2),
        Fu.Mem(count=2, latency=2),
        Fu.VecIntAlu(count=4, latency=4),
        Fu.VecIntMul(count=1, latency=10),
        Fu.VecIntDiv(count=1, latency=60),
        Fu.VecFpAlu(count=2, latency=10),
        Fu.VecFpFma(count=2, latency=12),
        Fu.VecFpDivSqrt(count=1, latency=50),
        Fu.VecMem(count=2, latency=2),
        Fu.VecPermute(count=2, latency=2),
    ])))),

    # ── LSU ports ─────────────────────────────────────────────────────────────
    # Single load + store port: worst-case memory bandwidth bottleneck.
    ("o3 w4 1ld-1st",       Config(width=4, backend=Backend.OutOfOrder(
        load_ports=1, store_ports=1,
    ))),
    # Four load + two store ports: unlikely to be port-limited.
    ("o3 w4 4ld-2st",       Config(width=4, backend=Backend.OutOfOrder(
        load_ports=4, store_ports=2,
    ))),

    # ── MSHRs ─────────────────────────────────────────────────────────────────
    # Blocking L1-D (mshr=0): every miss serialises, no coalescing possible.
    ("o3 w4 blocking-l1d",  Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1,
                                             mshr_count=0,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    # Single MSHR: a second miss while one is in flight must stall.
    ("o3 w4 mshr-1",        Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1,
                                             mshr_count=1,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    ("o3 w4 mshr-16",       Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1,
                                             mshr_count=16,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    # MSHRs on L2 as well: overlaps L2→RAM misses.
    ("o3 w4 l2-mshrs",      Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1,
                                             mshr_count=8,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=10, mshr_count=16))),

    # ── Cache hierarchy variants ───────────────────────────────────────────────
    # No L2: L1 misses go directly to RAM, high miss penalty.
    ("o3 w4 no-l2",         Config(width=4, backend=Backend.OutOfOrder(), l2=None)),
    # No caches at all: every fetch and data access hits RAM.
    ("o3 w4 no-caches",     Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=None, l1d=None, l2=None)),
    # L3 enabled: tests the optional third cache level.
    ("o3 w4 l3",            Config(width=4, backend=Backend.OutOfOrder(),
                                   l3=Cache("8MB", ways=16, latency=30))),
    # Tiny L1 (8KB): high miss rate, hammers miss-handling paths.
    ("o3 w4 tiny-l1",       Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("8KB",  ways=2, latency=1,
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("8KB",  ways=2, latency=1, mshr_count=4,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=32)))),
    # Large L1/L2 (M1-scale): near-zero miss rate, tests high-IPC ceiling.
    ("o3 w4 large-cache",   Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("128KB", ways=8, latency=2,
                                             prefetcher=Prefetcher.NextLine(degree=2)),
                                   l1d=Cache("128KB", ways=8, latency=2, mshr_count=16,
                                             prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
                                   l2=Cache("4MB", ways=16, latency=12))),
    # Direct-mapped: maximum conflict-miss rate for any set-indexed workload.
    ("o3 w4 direct-map",    Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=1, latency=1,
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=1, latency=1, mshr_count=4,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=1, latency=10))),
    # High associativity: 16-way L1, minimal conflict misses.
    ("o3 w4 high-assoc",    Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=16, latency=2,
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=16, latency=2, mshr_count=8,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=16, latency=12))),
    # High L1 access latency: load-use penalty dominates, stresses scheduling.
    ("o3 w4 slow-l1",       Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=4,
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=4, latency=4, mshr_count=8,
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=20))),

    # ── Prefetcher variants ────────────────────────────────────────────────────
    # No prefetcher: measures raw miss-rate without speculative fetches.
    ("o3 w4 no-pf",         Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8),
                                   l2=Cache("256KB", ways=8, latency=10))),
    ("o3 w4 stream-pf",     Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             prefetcher=Prefetcher.Stream()))),
    ("o3 w4 tagged-pf",     Config(width=4, backend=Backend.OutOfOrder(),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             prefetcher=Prefetcher.Tagged()))),
    # Aggressive next-line + stride: degree=4, large table.
    ("o3 w4 aggressive-pf", Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1,
                                             prefetcher=Prefetcher.NextLine(degree=4)),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             prefetcher=Prefetcher.Stride(degree=4, table_size=128)))),

    # ── Cache replacement policies ─────────────────────────────────────────────
    ("o3 w4 plru",          Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1,
                                             policy=ReplacementPolicy.PLRU(),
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             policy=ReplacementPolicy.PLRU(),
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=10,
                                            policy=ReplacementPolicy.PLRU()))),
    ("o3 w4 fifo",          Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1,
                                             policy=ReplacementPolicy.FIFO(),
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             policy=ReplacementPolicy.FIFO(),
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=10,
                                            policy=ReplacementPolicy.FIFO()))),
    ("o3 w4 mru",           Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1,
                                             policy=ReplacementPolicy.MRU(),
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             policy=ReplacementPolicy.MRU(),
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=10,
                                            policy=ReplacementPolicy.MRU()))),
    ("o3 w4 random",        Config(width=4, backend=Backend.OutOfOrder(),
                                   l1i=Cache("32KB", ways=4, latency=1,
                                             policy=ReplacementPolicy.Random(),
                                             prefetcher=Prefetcher.NextLine(degree=1)),
                                   l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                             policy=ReplacementPolicy.Random(),
                                             prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                   l2=Cache("256KB", ways=8, latency=10,
                                            policy=ReplacementPolicy.Random()))),

    # ── Memory controller ──────────────────────────────────────────────────────
    ("o3 w4 dram",          Config(width=4, backend=Backend.OutOfOrder(),
                                   memory_controller=MemoryController.DRAM())),
    # Slow DRAM: row-miss latency 200 cycles, stresses long-latency miss paths.
    ("o3 w4 dram-slow",     Config(width=4, backend=Backend.OutOfOrder(),
                                   memory_controller=MemoryController.DRAM(
                                       t_cas=30, t_ras=36, t_pre=12,
                                       row_miss_latency=200,
                                   ))),

    # ── Memory dependence predictors ────────────────────────────────────────
    ("o3 w4 mdp-blind",     Config(width=4, backend=Backend.OutOfOrder(),
                                   mem_dep_predictor=MemDepPredictor.Blind())),
    ("o3 w4 mdp-storeset",  Config(width=4, backend=Backend.OutOfOrder(),
                                   mem_dep_predictor=MemDepPredictor.StoreSet())),
    ("o3 w4 mdp-storeset-sm", Config(width=4, backend=Backend.OutOfOrder(),
                                   mem_dep_predictor=MemDepPredictor.StoreSet(
                                       ssit_size=256, lfst_size=64,
                                   ))),
    ("o3 w4 mdp-storeset-lg", Config(width=4, backend=Backend.OutOfOrder(),
                                   mem_dep_predictor=MemDepPredictor.StoreSet(
                                       ssit_size=8192, lfst_size=1024,
                                   ))),

    # ── TLB sizing ────────────────────────────────────────────────────────────
    # 4-entry TLB: almost every memory op triggers a page-table walk.
    ("o3 w4 small-tlb",     Config(width=4, backend=Backend.OutOfOrder(), tlb_size=4)),
    ("o3 w4 large-tlb",     Config(width=4, backend=Backend.OutOfOrder(), tlb_size=256)),

    # ── Reference machine configs ──────────────────────────────────────────────
    ("ref cortex-a72",      cortex_a72_config()),
    ("ref p550",            p550_config()),
    ("ref m1",              m1_config()),
    ("ref linux",           linux_config()),
]
# fmt: on
