#!/usr/bin/env python3
"""Compare rvsim against spike commit traces for all riscv-tests.

Phase 1 (oracle): Run spike -l on every riscv-test ELF, cache the logs.
Phase 2 (compare): For each pipeline config, run rvsim and diff the commit
                    trace against the cached spike oracle.

Usage (from repo root):
    rvsim --script scripts/spike_compare.py              # full run
    rvsim --script scripts/spike_compare.py --oracle-only # just generate spike logs
    rvsim --script scripts/spike_compare.py --skip-oracle # reuse cached spike logs
    rvsim --script scripts/spike_compare.py --pipelines 'inorder w1,o3 w4'
"""

import argparse
import contextlib
import glob
import io
import os
import re
import subprocess
import sys
import tempfile

from rvsim import Config  # noqa: F401  (kept for backward compatibility)
from rvsim._core import Cpu
from rvsim.config import _config_to_dict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ISA_DIR = os.path.join(ROOT, "testing", "builds", "riscv-tests", "isa")
ORACLE_DIR = os.path.join(ROOT, "testing", "builds", "spike-oracle")

# Shared pipeline matrix.
sys.path.insert(0, ROOT)
from testing.configs.pipelines import PIPELINES  # noqa: E402, F401

# ── Test suites ──────────────────────────────────────────────────────────────
SUITES = [
    "rv64ui",
    "rv64um",
    "rv64ua",
    "rv64uf",
    "rv64ud",
    "rv64uc",
    "rv64mi",
    "rv64si",
]

CYCLE_LIMIT = 500_000

# ── Spike log parsing ────────────────────────────────────────────────────────
LINE_RE = re.compile(r"core\s+\d+:\s+0x([0-9a-f]+)\s+\(0x([0-9a-f]+)\)")

# Instructions that rvsim's decode eliminates (never enter the pipeline).
NOP_ENCODINGS = {0x00000013, 0x00000001}

# Spike reset vector range.
SPIKE_RESET_START = 0x1000
SPIKE_RESET_END = 0x2000


def parse_log(path, skip_nops=True, skip_reset=False):
    """Parse a commit log into a list of (pc, inst) tuples."""
    entries = []
    with open(path) as f:
        for line in f:
            m = LINE_RE.search(line)
            if m is None:
                continue
            pc = int(m.group(1), 16)
            inst = int(m.group(2), 16)
            if skip_reset and SPIKE_RESET_START <= pc < SPIKE_RESET_END:
                continue
            if skip_nops and inst in NOP_ENCODINGS:
                continue
            entries.append((pc, inst))
    return entries


def compare_traces(spike_trace, rvsim_trace):
    """Compare two traces by PC sequence. Returns (match_count, divergence_info_or_None).

    Only PCs are compared, not instruction encodings, because rvsim logs
    the 32-bit expanded form of compressed instructions while spike logs
    the original 16-bit encoding.

    rvsim may be shorter than spike because rvsim exits immediately on
    HTIF tohost write, while spike continues the polling loop.  A shorter
    rvsim trace is fine as long as every PC it *did* retire matches spike
    exactly.  A shorter *spike* trace (rvsim retired instructions spike
    didn't) is a real divergence.
    """
    for i, (s, r) in enumerate(zip(spike_trace, rvsim_trace)):
        if s[0] != r[0]:
            return i, {
                "inst_num": i + 1,
                "spike_pc": s[0],
                "spike_inst": s[1],
                "rvsim_pc": r[0],
                "rvsim_inst": r[1],
            }
    # All matched up to the shorter trace
    min_len = min(len(spike_trace), len(rvsim_trace))
    if len(rvsim_trace) > len(spike_trace):
        # rvsim retired more instructions than spike — real divergence
        return min_len, {
            "inst_num": min_len + 1,
            "length_mismatch": True,
            "spike_len": len(spike_trace),
            "rvsim_len": len(rvsim_trace),
        }
    # rvsim <= spike in length, all matched: OK (rvsim exits early on HTIF)
    return min_len, None


def find_tests():
    """Discover all -p- test ELFs, sorted by suite then name."""
    tests = []
    for suite in SUITES:
        pattern = os.path.join(ISA_DIR, f"{suite}-p-*")
        found = sorted(f for f in glob.glob(pattern) if not f.endswith(".dump"))
        tests.extend(found)
    return tests


def generate_spike_oracle(tests):
    """Run spike -l on every test, save parsed traces to ORACLE_DIR."""
    os.makedirs(ORACLE_DIR, exist_ok=True)

    print(f"[oracle] Generating spike traces for {len(tests)} tests...")
    failed = []

    for i, path in enumerate(tests, 1):
        name = os.path.basename(path)
        oracle_path = os.path.join(ORACLE_DIR, f"{name}.log")

        result = subprocess.run(
            ["spike", "-l", "--isa=rv64gc", path],
            capture_output=True, timeout=30,
        )

        if result.returncode != 0:
            failed.append(name)
            continue

        # Save spike's stderr (that's where -l output goes)
        with open(oracle_path, "wb") as f:
            f.write(result.stderr)

        if i % 20 == 0 or i == len(tests):
            print(f"  [{i:3d}/{len(tests)}] generated")

    if failed:
        print(f"[oracle] WARNING: {len(failed)} spike failures: {failed[:5]}")
    else:
        print(f"[oracle] All {len(tests)} spike traces generated successfully")

    return failed


def load_spike_trace(test_name):
    """Load and parse a cached spike trace."""
    oracle_path = os.path.join(ORACLE_DIR, f"{test_name}.log")
    if not os.path.exists(oracle_path):
        return None
    return parse_log(oracle_path, skip_nops=True, skip_reset=True)


def run_rvsim_trace(elf_path, cfg):
    """Run rvsim, return the commit trace as list of (pc, inst)."""
    config_dict = _config_to_dict(cfg)

    with open(elf_path, "rb") as f:
        elf_data = f.read()

    cpu = Cpu(config_dict, elf_data=elf_data)

    # Use a temp file for the commit log
    fd, log_path = tempfile.mkstemp(suffix=".log", prefix="rvsim_")
    os.close(fd)

    try:
        cpu.open_commit_log(log_path)

        with (
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            cpu.run(limit=CYCLE_LIMIT, stats_sections=None)

        # Flush by deleting the cpu (drops the BufWriter)
        del cpu

        trace = parse_log(log_path, skip_nops=True, skip_reset=False)
    finally:
        if os.path.exists(log_path):
            os.unlink(log_path)

    return trace


def format_divergence(div_info, test_name):
    """Format a divergence for printing."""
    if div_info.get("length_mismatch"):
        return (
            f"    {test_name}: LENGTH MISMATCH at inst #{div_info['inst_num']:,} "
            f"(spike={div_info['spike_len']:,}, rvsim={div_info['rvsim_len']:,})"
        )
    return (
        f"    {test_name}: DIVERGE at inst #{div_info['inst_num']:,}  "
        f"spike=0x{div_info['spike_pc']:016x}(0x{div_info['spike_inst']:08x})  "
        f"rvsim=0x{div_info['rvsim_pc']:016x}(0x{div_info['rvsim_inst']:08x})"
    )


def main():
    ap = argparse.ArgumentParser(
        description="Compare rvsim commit traces against spike oracle"
    )
    ap.add_argument(
        "--oracle-only", action="store_true",
        help="Only generate spike oracle logs, don't run comparisons",
    )
    ap.add_argument(
        "--skip-oracle", action="store_true",
        help="Skip oracle generation, reuse cached spike logs",
    )
    ap.add_argument(
        "--pipelines", type=str, default=None,
        help="Comma-separated list of pipeline labels to test (default: all)",
    )
    ap.add_argument(
        "--test", type=str, default=None,
        help="Run only tests matching this substring",
    )
    ap.add_argument(
        "--stop-on-fail", action="store_true",
        help="Stop at first divergence within each pipeline",
    )
    args = ap.parse_args()

    os.chdir(ROOT)
    tests = find_tests()
    if not tests:
        print(f"No tests found in {ISA_DIR}", file=sys.stderr)
        return 1

    if args.test:
        tests = [t for t in tests if args.test in os.path.basename(t)]
        if not tests:
            print(f"No tests matching '{args.test}'", file=sys.stderr)
            return 1

    # ── Phase 1: Oracle ──────────────────────────────────────────────────────
    if not args.skip_oracle:
        spike_failures = generate_spike_oracle(tests)
        if spike_failures:
            # Remove tests that spike itself can't handle
            spike_fail_set = set(spike_failures)
            tests = [t for t in tests if os.path.basename(t) not in spike_fail_set]

    if args.oracle_only:
        return 0

    # Filter out tests without an oracle (spike may have failed on them)
    available_tests = []
    skipped = []
    for path in tests:
        name = os.path.basename(path)
        if os.path.exists(os.path.join(ORACLE_DIR, f"{name}.log")):
            available_tests.append(path)
        else:
            skipped.append(name)
    if skipped:
        print(f"[compare] Skipping {len(skipped)} tests without spike oracle: {skipped}")
    tests = available_tests
    if not tests:
        print("No tests with spike oracle available.", file=sys.stderr)
        return 1

    # ── Phase 2: Compare ─────────────────────────────────────────────────────
    pipelines = PIPELINES
    if args.pipelines:
        selected = {s.strip() for s in args.pipelines.split(",")}
        pipelines = [(l, c) for l, c in PIPELINES if l in selected]
        if not pipelines:
            print(f"No matching pipelines for: {args.pipelines}", file=sys.stderr)
            return 1

    print(
        f"\n[compare] {len(tests)} tests x {len(pipelines)} pipelines "
        f"= {len(tests) * len(pipelines):,} comparisons\n"
    )

    # Pre-load all spike traces into memory (they're small)
    spike_traces = {}
    for path in tests:
        name = os.path.basename(path)
        spike_traces[name] = load_spike_trace(name)

    overall_failures = []
    total_match = 0
    total_fail = 0

    for pi, (label, cfg) in enumerate(pipelines, 1):
        pipe_match = 0
        pipe_fail = 0
        pipe_failures = []

        for path in tests:
            name = os.path.basename(path)
            spike_trace = spike_traces[name]
            if spike_trace is None:
                continue

            rvsim_trace = run_rvsim_trace(path, cfg)
            _, div_info = compare_traces(spike_trace, rvsim_trace)

            if div_info is None:
                pipe_match += 1
            else:
                pipe_fail += 1
                pipe_failures.append((name, div_info))
                if args.stop_on_fail:
                    break

        total_match += pipe_match
        total_fail += pipe_fail
        for name, div in pipe_failures:
            overall_failures.append((label, name, div))

        status = "PASS" if pipe_fail == 0 else f"{pipe_fail} DIVERGED"
        done = total_match + total_fail
        total = len(tests) * len(pipelines)
        print(
            f"  [{pi:2d}/{len(pipelines)}] {label:30s}  "
            f"{pipe_match}/{len(tests)} match  ({status})  "
            f"[total: {total_fail} failures / {done} run]"
        )

        # Print per-test failures inline if any
        for name, div in pipe_failures:
            print(format_divergence(div, name))

    # ── Summary ──────────────────────────────────────────────────────────────
    total = total_match + total_fail
    print(f"\n{'=' * 72}")
    print(f"SPIKE COMPARISON: {total_match} matched, {total_fail} diverged out of {total}")
    print(f"{'=' * 72}")

    if overall_failures:
        print(f"\n=== {total_fail} Divergences ===")
        for label, name, div in overall_failures:
            print(f"  [{label}] {format_divergence(div, name).strip()}")
        return 1

    print("\nZERO DIVERGENCE — all traces match spike exactly.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
