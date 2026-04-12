#!/usr/bin/env python3
"""Run riscv-tests ISA compliance suite against rvsim.

Tests a large variety of pipeline, cache, FU, and memory configurations.
The full PIPELINES matrix lives in testing/configs/pipelines.py and is shared
with the riscof and vector runners — edit it once, all three runners pick up
the change.

Usage (from repo root):
    .venv/bin/python testing/run_riscv_tests.py
"""

import contextlib
import glob
import io
import os
import sys

from rvsim import Config, Simulator  # noqa: F401

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ISA_DIR = os.path.join(ROOT, "testing", "builds", "riscv-tests", "isa")

# Shared pipeline matrix.
sys.path.insert(0, ROOT)
from testing.configs.pipelines import PIPELINES  # noqa: E402

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
            "Run: make riscv-tests-build",
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
