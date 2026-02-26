#!/usr/bin/env python3
"""Run riscv-tests ISA compliance suite against rvsim.

Tests a large variety of pipeline, cache, FU, and memory configurations.

Usage (from repo root):
    rvsim --script scripts/run_riscv_tests.py
"""

import contextlib
import glob
import io
import os
import sys

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
    Simulator,
)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ISA_DIR = os.path.join(ROOT, "software", "riscv-tests", "isa")

# Pull in production benchmark configs to test the full reference machines.
sys.path.insert(0, os.path.join(ROOT, "scripts", "benchmarks"))
from cortex_a72.config import cortex_a72_config  # noqa: E402
from m1.config import m1_config  # noqa: E402
from p550.config import p550_config  # noqa: E402
from setup.boot_linux import config as linux_config

# Test suites we support (physical memory, no VM, "-p-" variants).
SUITES = [
    "rv64ui",  # RV64 integer
    "rv64um",  # RV64 multiply/divide
    "rv64ua",  # RV64 atomics
    "rv64uf",  # RV64 single-float
    "rv64ud",  # RV64 double-float
    "rv64uc",  # RV64 compressed
    "rv64mi",  # RV64 machine-mode
    "rv64si",  # RV64 supervisor-mode
]

CYCLE_LIMIT = 500_000

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

    # ── BTB / RAS sizing ──────────────────────────────────────────────────────
    # Tiny BTB: most targets alias, stresses cold-start and aliasing penalty.
    ("o3 w4 small-btb",     Config(width=4, backend=Backend.OutOfOrder(),
                                   btb_size=64, ras_size=4)),
    ("o3 w4 large-btb",     Config(width=4, backend=Backend.OutOfOrder(),
                                   btb_size=16384, ras_size=64)),

    # ── FU pool: one unit of each type ────────────────────────────────────────
    # Every FU class becomes a bottleneck; hits structural stall for all types.
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


def find_tests():
    """Discover all -p- test ELFs, sorted by suite then name."""
    tests = []
    for suite in SUITES:
        pattern = os.path.join(ISA_DIR, f"{suite}-p-*")
        found = sorted(f for f in glob.glob(pattern) if not f.endswith(".dump"))
        tests.extend(found)
    return tests


def run_test(path: str, cfg: Config) -> int:
    """Run a single test ELF with the given config. Returns exit code (0 = pass)."""
    sim = Simulator().config(cfg).binary(path)
    # Mute per-test simulator output (HTIF messages, trace, debug prints).
    with (
        contextlib.redirect_stdout(io.StringIO()),
        contextlib.redirect_stderr(io.StringIO()),
    ):
        return sim.run(limit=CYCLE_LIMIT, stats_sections=None)


def run_pipeline(label: str, cfg: Config, tests: list) -> tuple[int, list]:
    """Run all tests for one pipeline. Returns (passed, failed_list)."""
    passed = 0
    failed = []

    for path in tests:
        name = os.path.basename(path)
        rc = run_test(path, cfg)

        if rc == 0:
            passed += 1
        else:
            failed.append((name, rc))

    return passed, failed


def main():
    os.chdir(ROOT)
    tests = find_tests()
    if not tests:
        print(f"No tests found in {ISA_DIR}", file=sys.stderr)
        print(
            "Run: make RISCV_PREFIX=riscv64-elf- -C software/riscv-tests/isa XLEN=64",
            file=sys.stderr,
        )
        return 1

    print(
        f"Running {len(tests)} riscv-tests x {len(PIPELINES)} pipelines "
        f"(limit={CYCLE_LIMIT:,} cycles each)\n"
    )

    overall_failed = []
    total_pass = 0
    total_fail = 0
    total_tests = len(tests) * len(PIPELINES)

    for i, (label, cfg) in enumerate(PIPELINES, 1):
        passed, failed = run_pipeline(label, cfg, tests)
        total_pass += passed
        total_fail += len(failed)
        for name, rc in failed:
            overall_failed.append((label, name, rc))
        status = "PASS" if not failed else f"{len(failed)} FAIL"
        done = total_pass + total_fail
        print(
            f"  [{i:2d}/{len(PIPELINES)}] {label:30s}  {passed}/{len(tests)} passed  "
            f"({status})  [total: {total_fail} failures / {done} run]"
        )

    print(f"\n{'=' * 70}")
    print(f"TOTAL: {total_pass} passed, {total_fail} failed out of {total_tests}")
    print(f"{'=' * 70}")

    if overall_failed:
        print(f"\n=== {total_fail} Failures ===")
        for label, name, rc in overall_failed:
            print(f"  [{label}] {name} (exit={rc})")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
