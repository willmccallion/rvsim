"""Shared pipeline-config matrix used by every conformance runner.

A test runner imports `PIPELINES` from here and iterates over (label, cfg)
tuples. Each Config exercises a different rvsim configuration: pipeline width,
backend, branch predictor, ROB sizing, FU pool, cache hierarchy, prefetcher,
replacement policy, memory controller, mem-dep predictor, TLB, the two
configurations calibrated to measured hardware (cortex-a72, p550; see
tools/diag/calibration.py) and the Linux system.

If you add or remove a Config here, every runner picks it up automatically —
this is the single source of truth.
"""

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Coherence,
    Config,
    Fu,
    HomeAgent,
    Interconnect,
    MemDepPredictor,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
    presets,
)
from tools.diag import calibration

# The units of the matrix's base machine, scalar and vector.
_UNITS = [
    Fu.IntAlu(count=4, latency=1),
    Fu.IntMul(count=1, latency=3),
    Fu.IntDiv(count=1, latency=35),
    Fu.FpAdd(count=2, latency=4),
    Fu.FpMul(count=2, latency=5),
    Fu.FpFma(count=2, latency=5),
    Fu.FpDivSqrt(count=1, latency=21),
    Fu.Branch(count=2, latency=1),
    Fu.Mem(count=2, latency=1),
    Fu.VecIntAlu(count=1, latency=1),
    Fu.VecIntMul(count=1, latency=3),
    Fu.VecIntDiv(count=1, latency=20),
    Fu.VecFpAlu(count=1, latency=4),
    Fu.VecFpFma(count=1, latency=5),
    Fu.VecFpDivSqrt(count=1, latency=20),
    Fu.VecMem(count=1, latency=1),
    Fu.VecPermute(count=1, latency=1),
]


def _o3(**fields) -> Backend.OutOfOrder:
    """The base machine's out-of-order backend with ``fields`` in place."""
    base = {
        "rob_size": 128,
        "store_buffer_size": 32,
        "issue_queue_size": 32,
        "load_queue_size": 32,
        "load_ports": 2,
        "store_ports": 1,
        "prf_gpr_size": 256,
        "prf_fpr_size": 128,
        "prf_vpr_size": 64,
        "fu_config": Fu(_UNITS),
    }
    return Backend.OutOfOrder(**{**base, **fields})


def _in_order() -> Backend.InOrder:
    """The base machine's in-order backend."""
    return Backend.InOrder(fu_config=Fu(_UNITS))


def _row(**fields) -> Config:
    """The machine the matrix varies, with ``fields`` in place: a 4-wide
    out-of-order core with 32 KiB L1s, a 256 KiB L2 and fixed-latency
    memory. It is written out rather than taken from ``Config()``, so
    retuning the default machine leaves what the matrix covers alone."""
    base = {
        "width": 4,
        "backend": _o3(),
        "branch_predictor": BranchPredictor.Tage(),
        "btb_size": 4096,
        "btb_ways": 4,
        "l1i": Cache(
            "32KB", ways=4, latency=1, prefetcher=Prefetcher.NextLine(degree=1)
        ),
        "l1d": Cache(
            "32KB",
            ways=4,
            latency=1,
            prefetcher=Prefetcher.Stride(degree=1, table_size=64),
        ),
        "l2": Cache("256KB", ways=8, latency=10),
        "l3": None,
        "wcb_entries": 0,
        "load_prefetcher": None,
        "store_prefetcher": None,
        "memory_controller": MemoryController.Simple(),
        "bus_latency": 4,
        "cpu_clock_mhz": 2400,
        "clock_hz": 1_000_000_000,
        "l2_tlb_size": 0,
        "l2_tlb_ways": 4,
        "l2_tlb_latency": 4,
        "vlen": 128,
        "num_vec_lanes": None,
    }
    return Config(**{**base, **fields})


# fmt: off
PIPELINES = [
    # ── In-order widths ───────────────────────────────────────────────────────
    ("inorder w1",          _row(width=1, backend=_in_order())),
    ("inorder w2",          _row(width=2, backend=_in_order())),
    ("inorder w3",          _row(width=3, backend=_in_order())),
    ("inorder w4",          _row(width=4, backend=_in_order())),

    # ── O3 widths ─────────────────────────────────────────────────────────────
    ("o3 w1",               _row(width=1, backend=_o3())),
    ("o3 w2",               _row(width=2, backend=_o3())),
    ("o3 w3",               _row(width=3, backend=_o3())),
    ("o3 w4",               _row(width=4, backend=_o3())),
    ("o3 w8",               _row(width=8, backend=_o3())),

    # ── ROB / IQ / buffer sizing ──────────────────────────────────────────────
    # Smallest viable machine: structures fill immediately, constant flush/re-fill.
    ("o3 w4 small-rob",     _row(width=4, backend=_o3(
        rob_size=16, issue_queue_size=8,
        store_buffer_size=4, load_queue_size=8,
        prf_gpr_size=64, prf_fpr_size=64,
    ))),
    # Oversized: structures never fill, exposes wakeup correctness at distance.
    ("o3 w4 large-rob",     _row(width=4, backend=_o3(
        rob_size=256, issue_queue_size=128,
        store_buffer_size=64, load_queue_size=64,
        prf_gpr_size=384, prf_fpr_size=256,
    ))),
    # PRF exactly at the architectural minimum: rob_size + 32 each.
    ("o3 w4 tight-prf",     _row(width=4, backend=_o3(
        rob_size=64, prf_gpr_size=96, prf_fpr_size=96,
    ))),
    # Wide OOO window on a single-issue pipe: does out-of-order help at w=1?
    ("o3 w1 large-rob",     _row(width=1, backend=_o3(
        rob_size=128, issue_queue_size=64,
    ))),

    # ── Branch predictors ─────────────────────────────────────────────────────
    ("o3 w4 static",        _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.Static())),
    ("o3 w4 gshare",        _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.GShare())),
    ("o3 w4 tournament",    _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.Tournament())),
    ("o3 w4 perceptron",    _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.Perceptron())),
    # TAGE with more banks and larger tables than the default.
    ("o3 w4 tage-wide",     _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.Tage(
                                     num_banks=6, table_size=4096,
                                     reset_interval=2000,
                                     history_lengths=[5, 15, 44, 130, 320, 800],
                                     tag_widths=[9, 9, 10, 10, 11, 11],
                                 ))),
    # SC-L-TAGE with default parameters.
    ("o3 w4 sc-l-tage",     _row(width=4, backend=_o3(),
                                 branch_predictor=BranchPredictor.ScLTage())),
    # SC-L-TAGE with larger tables (matches linux config scale).
    ("o3 w4 sc-l-tage-wide", _row(width=4, backend=_o3(),
                                   branch_predictor=BranchPredictor.ScLTage(
                                     loop_log_size=9,
                                     ittage_num_banks=8, ittage_table_size=512,
                                   ))),

    # ── BTB / RAS sizing ──────────────────────────────────────────────────────
    # Tiny BTB: most targets alias, stresses cold-start and aliasing penalty.
    ("o3 w4 small-btb",     _row(width=4, backend=_o3(),
                                 btb_size=64, ras_size=4)),
    ("o3 w4 large-btb",     _row(width=4, backend=_o3(),
                                 btb_size=16384, ras_size=64)),

    # ── FU pool: one unit of each type ────────────────────────────────────────
    # Every FU class becomes a bottleneck; hits structural stall for all types.
    # Vector FUs are required: when fu_config is overridden, the rvsim FU pool
    # creates ONLY the units listed here, so omitting Vec* would deadlock any
    # vector op. Each variant scales scalar and vector pools together.
    ("o3 w4 fu-minimal",    _row(width=4, backend=_o3(fu_config=Fu([
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
    ("o3 w4 fu-mid",        _row(width=4, backend=_o3(fu_config=Fu([
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
    ("o3 w4 fu-wide",       _row(width=4, backend=_o3(fu_config=Fu([
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
    ("o3 w4 fu-slow",       _row(width=4, backend=_o3(fu_config=Fu([
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
    ("o3 w4 1ld-1st",       _row(width=4, backend=_o3(
        load_ports=1, store_ports=1,
    ))),
    # Four load + two store ports: unlikely to be port-limited.
    ("o3 w4 4ld-2st",       _row(width=4, backend=_o3(
        load_ports=4, store_ports=2,
    ))),

    # ── MSHRs ─────────────────────────────────────────────────────────────────
    # Blocking L1-D: one miss and one writeback in flight at a time.
    ("o3 w4 blocking-l1d",  _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1,
                                           mshr_count=1, write_buffers=1,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    # Single MSHR: a second miss while one is in flight must stall.
    ("o3 w4 mshr-1",        _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1,
                                           mshr_count=1,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    ("o3 w4 mshr-16",       _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1,
                                           mshr_count=16,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)))),
    # MSHRs on L2 as well: overlaps L2→RAM misses.
    ("o3 w4 l2-mshrs",      _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1,
                                           mshr_count=8,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=10, mshr_count=16))),

    # ── Cache hierarchy variants ───────────────────────────────────────────────
    # No L2: L1 misses go directly to RAM, high miss penalty.
    ("o3 w4 no-l2",         _row(width=4, backend=_o3(), l2=None)),
    # No caches at all: every fetch and data access hits RAM.
    ("o3 w4 no-caches",     _row(width=4, backend=_o3(),
                                 l1i=None, l1d=None, l2=None)),
    # L3 enabled: tests the optional third cache level.
    ("o3 w4 l3",            _row(width=4, backend=_o3(),
                                 l3=Cache("8MB", ways=16, latency=30))),
    # Tiny L1 (8KB): high miss rate, hammers miss-handling paths.
    ("o3 w4 tiny-l1",       _row(width=4, backend=_o3(),
                                 l1i=Cache("8KB",  ways=2, latency=1,
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("8KB",  ways=2, latency=1, mshr_count=4,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=32)))),
    # Large L1/L2 (M1-scale): near-zero miss rate, tests high-IPC ceiling.
    ("o3 w4 large-cache",   _row(width=4, backend=_o3(),
                                 l1i=Cache("128KB", ways=8, latency=2,
                                           prefetcher=Prefetcher.NextLine(degree=2)),
                                 l1d=Cache("128KB", ways=8, latency=2, mshr_count=16,
                                           prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
                                 l2=Cache("4MB", ways=16, latency=12))),
    # Direct-mapped: maximum conflict-miss rate for any set-indexed workload.
    ("o3 w4 direct-map",    _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=1, latency=1,
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=1, latency=1, mshr_count=4,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=1, latency=10))),
    # High associativity: 16-way L1, minimal conflict misses.
    ("o3 w4 high-assoc",    _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=16, latency=2,
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=16, latency=2, mshr_count=8,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=16, latency=12))),
    # High L1 access latency: load-use penalty dominates, stresses scheduling.
    ("o3 w4 slow-l1",       _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=4,
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=4, latency=4, mshr_count=8,
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=20))),

    # ── Prefetcher variants ────────────────────────────────────────────────────
    # No prefetcher: measures raw miss-rate without speculative fetches.
    ("o3 w4 no-pf",         _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8),
                                 l2=Cache("256KB", ways=8, latency=10))),
    ("o3 w4 stream-pf",     _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           prefetcher=Prefetcher.Stream()))),
    ("o3 w4 tagged-pf",     _row(width=4, backend=_o3(),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           prefetcher=Prefetcher.Tagged()))),
    # Aggressive next-line + stride: degree=4, large table.
    ("o3 w4 aggressive-pf", _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1,
                                           prefetcher=Prefetcher.NextLine(degree=4)),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           prefetcher=Prefetcher.Stride(degree=4, table_size=128)))),

    # ── Cache replacement policies ─────────────────────────────────────────────
    ("o3 w4 plru",          _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1,
                                           policy=ReplacementPolicy.Plru(),
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           policy=ReplacementPolicy.Plru(),
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=10,
                                          policy=ReplacementPolicy.Plru()))),
    ("o3 w4 fifo",          _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1,
                                           policy=ReplacementPolicy.Fifo(),
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           policy=ReplacementPolicy.Fifo(),
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=10,
                                          policy=ReplacementPolicy.Fifo()))),
    ("o3 w4 mru",           _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1,
                                           policy=ReplacementPolicy.Mru(),
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           policy=ReplacementPolicy.Mru(),
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=10,
                                          policy=ReplacementPolicy.Mru()))),
    ("o3 w4 random",        _row(width=4, backend=_o3(),
                                 l1i=Cache("32KB", ways=4, latency=1,
                                           policy=ReplacementPolicy.Random(),
                                           prefetcher=Prefetcher.NextLine(degree=1)),
                                 l1d=Cache("32KB", ways=4, latency=1, mshr_count=8,
                                           policy=ReplacementPolicy.Random(),
                                           prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
                                 l2=Cache("256KB", ways=8, latency=10,
                                          policy=ReplacementPolicy.Random()))),

    # ── Memory controller ──────────────────────────────────────────────────────
    ("o3 w4 dram",          _row(width=4, backend=_o3(),
                                 memory_controller=MemoryController.Dram())),
    # Slow DRAM: a 78-cycle row conflict stresses long-latency miss paths.
    ("o3 w4 dram-slow",     _row(width=4, backend=_o3(),
                                 memory_controller=MemoryController.Dram(
                                     t_cas=30, t_ras=36, t_pre=12,
                                 ))),
    # DDR5-4800 command-level controller at a 1:1 clock ratio.
    ("o3 w4 ddr5",          _row(width=4, backend=_o3(),
                                 memory_controller=MemoryController.Ddr5())),
    ("inorder w4 ddr5",     _row(width=4, backend=_in_order(),
                                 memory_controller=MemoryController.Ddr5())),
    # 3 GHz core over DDR5-5600 with same-bank refresh, power-down and
    # patrol scrubbing: every clock-domain and policy path exercised.
    ("o3 w4 ddr5-5600",     _row(width=4, backend=_o3(),
                                 cpu_clock_mhz=3000,
                                 memory_controller=MemoryController.Ddr5(
                                     speed_bin="Ddr5_5600B", refresh="SameBank",
                                     power_down_idle_ns=200, ecc="SecDed",
                                     patrol_scrub_ns=100_000,
                                 ))),

    # ── Memory dependence predictors ────────────────────────────────────────
    ("o3 w4 mdp-blind",     _row(width=4, backend=_o3(),
                                 mem_dep_predictor=MemDepPredictor.Blind())),
    ("o3 w4 mdp-storeset",  _row(width=4, backend=_o3(),
                                 mem_dep_predictor=MemDepPredictor.StoreSet())),
    ("o3 w4 mdp-storeset-sm", _row(width=4, backend=_o3(),
                                   mem_dep_predictor=MemDepPredictor.StoreSet(
                                     ssit_size=256, lfst_size=64,
                                   ))),
    ("o3 w4 mdp-storeset-lg", _row(width=4, backend=_o3(),
                                   mem_dep_predictor=MemDepPredictor.StoreSet(
                                     ssit_size=8192, lfst_size=1024,
                                   ))),

    # ── TLB sizing ────────────────────────────────────────────────────────────
    # 4-entry TLB: almost every memory op triggers a page-table walk.
    ("o3 w4 small-tlb",     _row(width=4, backend=_o3(), tlb_size=4)),
    ("o3 w4 large-tlb",     _row(width=4, backend=_o3(), tlb_size=256)),

    # ── Paging-mode caps ──────────────────────────────────────────────────────
    # Force the SATP writer to coerce anything stronger to Bare. Most riscv-
    # tests use Sv39, so only the Linux-boot config and any kernel that
    # probes Sv57/Sv48 first will see different behaviour. Default is sv57
    # (no cap).
    ("o3 w4 sv39-only",     _row(width=4, backend=_o3(), paging_mode_max="Sv39")),
    ("o3 w4 sv48-cap",      _row(width=4, backend=_o3(), paging_mode_max="Sv48")),
    ("inorder w4 sv39-only", _row(width=4, backend=_in_order(), paging_mode_max="Sv39")),
    ("inorder w4 sv48-cap", _row(width=4, backend=_in_order(), paging_mode_max="Sv48")),

    # ── Reference machine configs ──────────────────────────────────────────────
    ("ref cortex-a72",      calibration.cortex_a72()),
    # ── Coherence fabric (only matters with hart_count > 1) ──────────────────
    ("smp broadcast",       _row(width=4, backend=_o3(),
                                 coherence=Coherence(home_agent=HomeAgent.Broadcast()))),
    ("smp ring",            _row(width=4, backend=_o3(),
                                 coherence=Coherence(interconnect=Interconnect.Ring(hop_latency=1)))),
    ("smp mesh",            _row(width=4, backend=_o3(),
                                 coherence=Coherence(interconnect=Interconnect.Mesh()))),
    ("smp torus",           _row(width=4, backend=_o3(),
                                 coherence=Coherence(interconnect=Interconnect.Torus()))),
    ("smp hypercube",       _row(width=4, backend=_o3(),
                                 coherence=Coherence(interconnect=Interconnect.Hypercube(hop_latency=3)))),
    ("smp tiny-filter",     _row(width=4, backend=_o3(),
                                 coherence=Coherence(home_agent=HomeAgent.SnoopFilter(capacity_factor=0.05, ways=2)))),

    ("ref p550",            calibration.p550()),
    ("ref linux",           presets.linux()),
]
# fmt: on

# The configs `--smoke` runs every riscv-test on: both backends at the
# narrowest and widest widths, no caches, an L3, every prefetcher, DDR5,
# capped paging modes, a coherent multi-core fabric, and the two calibrated
# configurations.
SMOKE_PIPELINES = [
    "inorder w1",
    "inorder w4",
    "o3 w1",
    "o3 w4",
    "o3 w8",
    "o3 w4 no-caches",
    "o3 w4 l3",
    "o3 w4 aggressive-pf",
    "o3 w4 ddr5",
    "o3 w4 sv39-only",
    "inorder w4 sv39-only",
    "smp mesh",
    "ref cortex-a72",
    "ref p550",
]

# The configs `--smoke` runs the vector sample on: one of each backend.
VECTOR_SMOKE_PIPELINES = ["inorder w4", "o3 w4"]
