#!/usr/bin/env python3
"""Compare rvsim against spike commit traces for all riscv-tests.

Phase 1 (oracle): Run spike -l on every riscv-test ELF, cache the logs.
Phase 2 (compare): For each pipeline config, run rvsim and diff the commit
                    trace against the cached spike oracle.

Usage (from repo root):
    .venv/bin/python tests/conformance/spike_compare.py              # full run
    .venv/bin/python tests/conformance/spike_compare.py --oracle-only # just generate spike logs
    .venv/bin/python tests/conformance/spike_compare.py --skip-oracle # reuse cached spike logs
    .venv/bin/python tests/conformance/spike_compare.py --pipelines 'inorder w1,o3 w4'
"""

import argparse
import contextlib
import glob
import io
import os
import subprocess
import sys
import tempfile

from rvsim._core import Simulator
from rvsim.config._config import _config_to_dict

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ISA_DIR = os.path.join(ROOT, "tests", "builds", "riscv-tests", "isa")
ORACLE_DIR = os.path.join(ROOT, "tests", "builds", "spike-oracle")

# Shared pipeline matrix.
sys.path.insert(0, ROOT)
from tests.conformance.common import SPIKE, compare_traces, parse_log, select_pipelines
from tests.conformance.configs.pipelines import PIPELINES

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
            [SPIKE, "-l", "--isa=rv64gc", path],
            capture_output=True,
            timeout=30,
            check=False,
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

    cpu = Simulator(config_dict, elf_data=elf_data)

    # Use a temp file for the commit log
    fd, log_path = tempfile.mkstemp(suffix=".log", prefix="rvsim_")
    os.close(fd)

    try:
        cpu.open_commit_log(log_path)

        with (
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            cpu.run(limit=CYCLE_LIMIT)

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


def parse_args():
    ap = argparse.ArgumentParser(
        description="Compare rvsim commit traces against spike oracle"
    )
    ap.add_argument(
        "--oracle-only",
        action="store_true",
        help="Only generate spike oracle logs, don't run comparisons",
    )
    ap.add_argument(
        "--skip-oracle",
        action="store_true",
        help="Skip oracle generation, reuse cached spike logs",
    )
    ap.add_argument(
        "--pipelines",
        type=str,
        default=None,
        help="Comma-separated list of pipeline labels to test (default: all)",
    )
    ap.add_argument(
        "--test",
        type=str,
        default=None,
        help="Run only tests matching this substring",
    )
    ap.add_argument(
        "--stop-on-fail",
        action="store_true",
        help="Stop at first divergence within each pipeline",
    )
    return ap.parse_args()


def with_oracle(tests):
    """The `tests` that have a spike oracle log, naming those that do not."""
    available = [
        path
        for path in tests
        if os.path.exists(os.path.join(ORACLE_DIR, f"{os.path.basename(path)}.log"))
    ]
    missing = [os.path.basename(path) for path in tests if path not in available]
    if missing:
        print(
            f"[compare] Skipping {len(missing)} tests without spike oracle: {missing}"
        )
    return available


def compare_pipeline(tests, spike_traces, cfg, stop_on_fail):
    """The (test, divergence) of each of `tests` whose rvsim trace on `cfg`
    leaves spike's, and how many matched."""
    matched, failures = 0, []
    for path in tests:
        name = os.path.basename(path)
        _, divergence = compare_traces(spike_traces[name], run_rvsim_trace(path, cfg))
        if divergence is None:
            matched += 1
            continue
        failures.append((name, divergence))
        if stop_on_fail:
            break
    return matched, failures


def compare_all(tests, pipelines, stop_on_fail):
    """Compares every test on every pipeline, printing each pipeline's
    result as it finishes; returns (matched, [(label, test, divergence)])."""
    spike_traces = {
        os.path.basename(p): load_spike_trace(os.path.basename(p)) for p in tests
    }
    tests = [p for p in tests if spike_traces[os.path.basename(p)] is not None]
    total_match, failures = 0, []
    for pi, (label, cfg) in enumerate(pipelines, 1):
        matched, pipe_failures = compare_pipeline(
            tests, spike_traces, cfg, stop_on_fail
        )
        total_match += matched
        failures += [(label, name, div) for name, div in pipe_failures]
        status = "PASS" if not pipe_failures else f"{len(pipe_failures)} DIVERGED"
        print(
            f"  [{pi:2d}/{len(pipelines)}] {label:30s}  "
            f"{matched}/{len(tests)} match  ({status})  "
            f"[total: {len(failures)} failures / {total_match + len(failures)} run]"
        )
        for name, div in pipe_failures:
            print(format_divergence(div, name))
    return total_match, failures


def print_summary(total_match, failures):
    print(f"\n{'=' * 72}")
    print(
        f"SPIKE COMPARISON: {total_match} matched, {len(failures)} diverged "
        f"out of {total_match + len(failures)}"
    )
    print(f"{'=' * 72}")
    if failures:
        print(f"\n=== {len(failures)} Divergences ===")
        for label, name, div in failures:
            print(f"  [{label}] {format_divergence(div, name).strip()}")
    else:
        print("\nZERO DIVERGENCE — all traces match spike exactly.")


def main():
    args = parse_args()
    os.chdir(ROOT)
    tests = [
        t for t in find_tests() if not args.test or args.test in os.path.basename(t)
    ]
    if not tests:
        print(f"No tests in {ISA_DIR} match '{args.test or ''}'", file=sys.stderr)
        return 1
    if not args.skip_oracle:
        spike_failures = set(generate_spike_oracle(tests))
        tests = [t for t in tests if os.path.basename(t) not in spike_failures]
    if args.oracle_only:
        return 0
    tests = with_oracle(tests)
    if not tests:
        print("No tests with spike oracle available.", file=sys.stderr)
        return 1
    pipelines = select_pipelines(PIPELINES, args.pipelines)
    print(
        f"\n[compare] {len(tests)} tests x {len(pipelines)} pipelines "
        f"= {len(tests) * len(pipelines):,} comparisons\n"
    )
    total_match, failures = compare_all(tests, pipelines, args.stop_on_fail)
    print_summary(total_match, failures)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
