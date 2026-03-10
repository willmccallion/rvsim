#!/usr/bin/env python3
"""
Debug script that boots Linux and captures state around the crash when
entering userspace (/sbin/init).

Usage:
    # Default: boot O3, stop ~1M cycles before expected crash, trace the rest:
    rvsim scripts/debug/debug_userspace.py

    # Use inorder backend:
    rvsim scripts/debug/debug_userspace.py --inorder

    # Set the cycle at which to start tracing (default: 1M before crash):
    rvsim scripts/debug/debug_userspace.py --trace-at 119000000

    # Write a commit log to disk (every retired instruction from trace point):
    rvsim scripts/debug/debug_userspace.py --commit-log /tmp/init.log

    # Dump N instructions of context before crash (default 200):
    rvsim scripts/debug/debug_userspace.py --context 500

    # Save checkpoint at the trace point:
    rvsim scripts/debug/debug_userspace.py --save-checkpoint /tmp/pre_crash.bin

    # Enable Rust-level tracing for specific subsystems:
    RUST_LOG=rvsim::trap=trace rvsim scripts/debug/debug_userspace.py
    RUST_LOG=rvsim=trace rvsim scripts/debug/debug_userspace.py
"""

import argparse
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
from rvsim._core import disassemble


def repo_root():
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


# ── Configs ──────────────────────────────────────────────────────────────────

def common_kwargs():
    """Shared system / memory config for both backends."""
    return dict(
        l1i=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.NextLine(degree=4), mshr_count=8),
        l1d=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.Stride(degree=4, table_size=256), mshr_count=16),
        l2=Cache(size="2MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(),
                 latency=8, prefetcher=Prefetcher.Stream(degree=8), mshr_count=32),
        l3=Cache(size="16MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(),
                 latency=24, prefetcher=Prefetcher.Tagged(degree=4), mshr_count=64),
        inclusion_policy=Cache.Inclusive(),
        wcb_entries=16,
        ram_size="256MB", tlb_size=256, l2_tlb_size=2048, l2_tlb_ways=8, l2_tlb_latency=3,
        memory_controller=MemoryController.Simple(),
        ram_base=0x80000000, uart_base=0x10000000, disk_base=0x10001000,
        clint_base=0x02000000, syscon_base=0x00100000, kernel_offset=0x200000,
        bus_width=8, bus_latency=1, clint_divider=1,
    )


def o3_config():
    return Config(
        width=8,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=8, table_size=8192, loop_table_size=1024,
            reset_interval=500_000,
            history_lengths=[5, 11, 22, 44, 89, 178, 356, 712],
            tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
        ),
        btb_size=16384, btb_ways=8, ras_size=128,
        backend=Backend.OutOfOrder(
            rob_size=256, store_buffer_size=64, issue_queue_size=96,
            load_queue_size=64, load_ports=4, store_ports=2,
            prf_gpr_size=512, prf_fpr_size=256,
            fu_config=Fu([
                Fu.IntAlu(count=6, latency=1), Fu.IntMul(count=2, latency=3),
                Fu.IntDiv(count=2, latency=20), Fu.FpAdd(count=4, latency=4),
                Fu.FpMul(count=4, latency=5), Fu.FpFma(count=4, latency=5),
                Fu.FpDivSqrt(count=2, latency=21), Fu.Branch(count=4, latency=1),
                Fu.Mem(count=4, latency=1),
            ]),
        ),
        **common_kwargs(),
    )


def inorder_config():
    return Config(width=1, backend=Backend.InOrder(), **common_kwargs())


# Known approximate crash cycles (from previous runs).
# O3: ~120.4M, InOrder: will be different. These are starting estimates;
# use --trace-at to override.
CRASH_CYCLE_O3 = 120_443_361
CRASH_CYCLE_INORDER = 250_000_000  # conservative estimate


# ── Register dump ────────────────────────────────────────────────────────────

ABI_NAMES = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2",
    "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5",
    "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7",
    "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
]


def dump_regs(cpu):
    print("\n── GPR state ──")
    for i in range(0, 32, 4):
        parts = []
        for j in range(4):
            r = i + j
            val = cpu.regs[r]
            parts.append(f"  {ABI_NAMES[r]:>4}(x{r:<2d}) = {val:#018x}")
        print("".join(parts))


def dump_csrs(cpu):
    print("\n── Key CSRs ──")
    names = [
        "mstatus", "sstatus", "mepc", "sepc", "mcause", "scause",
        "mtval", "stval", "mtvec", "stvec", "satp", "mideleg", "medeleg",
    ]
    for name in names:
        try:
            val = cpu.csrs[name]
            print(f"  {name:>12s} = {val:#018x}")
        except Exception:
            pass


def dump_pc_trace(cpu, n):
    trace = cpu.pc_trace
    if not trace:
        print("\n  (no committed instructions in trace)")
        return
    start = max(0, len(trace) - n)
    print(f"\n── Last {len(trace) - start} committed instructions ──")
    for pc, raw in trace[start:]:
        asm = disassemble(raw)
        print(f"  {pc:#018x}:  {raw:08x}  {asm}")


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    root = repo_root()
    linux_dir = os.path.join(root, "software", "linux")
    image = os.path.join(linux_dir, "output", "Image")
    disk = os.path.join(linux_dir, "output", "disk.img")
    dtb = os.path.join(linux_dir, "system.dtb")

    for path, label in [(image, "kernel Image"), (disk, "disk image")]:
        if not os.path.exists(path):
            print(f"Error: {label} not found at {path}", file=sys.stderr)
            print("Run boot_linux.py first to build the artifacts.", file=sys.stderr)
            return 1

    ap = argparse.ArgumentParser(description="Debug Linux userspace entry")
    ap.add_argument("--inorder", action="store_true", help="Use in-order backend")
    ap.add_argument("--trace-at", type=int, default=None,
                    help="Cycle at which to enable tracing (default: 1M before expected crash)")
    ap.add_argument("--trace-window", type=int, default=1_000_000,
                    help="How many cycles before expected crash to start tracing (default: 1M)")
    ap.add_argument("--commit-log", metavar="PATH",
                    help="Write full commit log to file from trace point onward")
    ap.add_argument("--context", type=int, default=200,
                    help="Number of instructions to show before crash (default: 200)")
    ap.add_argument("--limit", type=int, default=500_000_000,
                    help="Max total cycles (default: 500M)")
    ap.add_argument("--save-checkpoint", metavar="PATH",
                    help="Save a checkpoint at the trace point")
    args = ap.parse_args()

    cfg = inorder_config() if args.inorder else o3_config()
    backend_name = "InOrder" if args.inorder else "O3"
    expected_crash = CRASH_CYCLE_INORDER if args.inorder else CRASH_CYCLE_O3

    if args.trace_at is not None:
        trace_at = args.trace_at
    else:
        trace_at = max(0, expected_crash - args.trace_window)

    print(f"[debug] Booting Linux ({backend_name} backend)...", file=sys.stderr)
    print(f"[debug] Will trace from cycle {trace_at:,} "
          f"(expected crash ~{expected_crash:,})", file=sys.stderr)

    sim = Simulator().config(cfg).kernel(image).disk(disk)
    if os.path.isfile(dtb):
        sim.dtb(dtb)
    cpu = sim.build()

    # Phase 1: Run silently to the trace point.
    print(f"[debug] Phase 1: running {trace_at:,} cycles silently...", file=sys.stderr)
    try:
        exit_code = cpu.run(limit=trace_at, stats_sections=None)
    except Exception as e:
        print(f"\n[debug] Crashed during phase 1: {e}", file=sys.stderr)
        dump_state(cpu, args.context)
        return 1

    if exit_code is not None:
        print(f"[debug] Simulation exited with code {exit_code} at cycle "
              f"{cpu.stats['cycles']:,} (before trace point)", file=sys.stderr)
        dump_state(cpu, args.context)
        return 1

    print(f"\n[debug] Reached trace point at cycle {cpu.stats['cycles']:,}", file=sys.stderr)
    print(f"[debug]   PC = {cpu.pc:#018x}", file=sys.stderr)
    print(f"[debug]   privilege = {cpu.privilege}", file=sys.stderr)

    # Save checkpoint if requested.
    if args.save_checkpoint:
        cpu.save(args.save_checkpoint)
        print(f"[debug] Checkpoint saved to {args.save_checkpoint}", file=sys.stderr)

    # Phase 2: Enable tracing and run until crash / exit / limit.
    cpu.trace = True

    if args.commit_log:
        try:
            cpu.open_commit_log(args.commit_log)
            print(f"[debug] Commit log: {args.commit_log}", file=sys.stderr)
        except Exception as e:
            print(f"[debug] Warning: could not open commit log: {e}", file=sys.stderr)

    remaining = max(0, args.limit - trace_at)
    print(f"[debug] Phase 2: tracing enabled, running up to {remaining:,} more cycles...",
          file=sys.stderr)

    try:
        exit_code = cpu.run(limit=remaining, stats_sections=None)
    except Exception as e:
        print(f"\n[debug] Simulation exception: {e}", file=sys.stderr)
        exit_code = None

    dump_state(cpu, args.context)
    return 0


def dump_state(cpu, context):
    """Dump full machine state for debugging."""
    cycles = cpu.stats["cycles"]
    print(f"\n{'=' * 72}", file=sys.stderr)
    print(f"[debug] Final state at cycle {cycles:,}", file=sys.stderr)
    print(f"[debug]   PC = {cpu.pc:#018x}", file=sys.stderr)
    print(f"[debug]   privilege = {cpu.privilege}", file=sys.stderr)
    dump_regs(cpu)
    dump_csrs(cpu)
    dump_pc_trace(cpu, context)


if __name__ == "__main__":
    sys.exit(main())
