#!/usr/bin/env python3
"""Run riscv-tests ISA compliance suite against rvsim.

Usage (from repo root):
    rvsim --script scripts/run_riscv_tests.py
"""

import glob
import os
import sys

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ISA_DIR = os.path.join(ROOT, "software", "riscv-tests", "isa")

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


def run_test(path: str) -> int:
    """Run a single test ELF. Returns exit code (0 = pass)."""
    sim = (
        Simulator()
        .with_config(Config(width=1))
        .binary(path)
    )
    return sim.run(limit=CYCLE_LIMIT, stats_sections=None)


def main():
    os.chdir(ROOT)
    tests = find_tests()
    if not tests:
        print(f"No tests found in {ISA_DIR}", file=sys.stderr)
        print("Run: make RISCV_PREFIX=riscv64-elf- -C software/riscv-tests/isa XLEN=64", file=sys.stderr)
        return 1

    print(f"Running {len(tests)} riscv-tests (limit={CYCLE_LIMIT:,} cycles each)\n")

    passed = 0
    failed = []

    for path in tests:
        name = os.path.basename(path)
        sys.stdout.write(f"  {name:40s} ")
        sys.stdout.flush()

        rc = run_test(path)

        if rc == 0:
            print("PASS")
            passed += 1
        else:
            print(f"FAIL (exit={rc})")
            failed.append((name, rc))

    print(f"\nResults: {passed} passed, {len(failed)} failed out of {passed + len(failed)} run")
    if failed:
        for name, rc in failed:
            print(f"  FAIL: {name} (exit={rc})")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
