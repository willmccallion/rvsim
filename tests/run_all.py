#!/usr/bin/env python3
"""Unified entry point: every conformance suite × every PIPELINES config.

Runs in order:
  1. tests/conformance/riscv_tests.py     — riscv-tests across all PIPELINES
  2. tests/conformance/vector_tests.py    — RVV cosim across all PIPELINES
  3. tests/conformance/multicore_tests.py — multi-hart programs at 2 and 4 harts
  4. tests/conformance/lockstep.py        — programs replayed on spike in lockstep

With --smoke, riscv-tests run on SMOKE_PIPELINES, the vector suite runs the
sample `make vector-smoke-build` builds on VECTOR_SMOKE_PIPELINES, the
multicore suite runs 4 harts on fewer configs, and lockstep is left out: a
few minutes on four cores, and what CI runs.

Each child runner streams its own JSON to tests/builds/results/. This
script tails their stdout, captures pass/fail counts from those JSONs at
the end, prints a unified summary, and exits non-zero if any suite failed.

This is the headline "did I break anything?" command. Expect several CPU-
hours on a workstation; an SSD and a lot of cores help.

Usage:
    .venv/bin/python tests/run_all.py
    .venv/bin/python tests/run_all.py --skip vector
    .venv/bin/python tests/run_all.py --vlen 256
    .venv/bin/python tests/run_all.py --pipelines 'inorder w1,o3 w4'
"""

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PYTHON = os.path.join(ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
RESULTS_DIR = os.path.join(ROOT, "tests", "builds", "results")
VECTOR_SMOKE_BUILD = os.path.join(ROOT, "tests", "builds", "vector-smoke")


def fmt_seconds(s):
    if s < 60:
        return f"{s:.0f}s"
    if s < 3600:
        return f"{s / 60:.1f}m"
    return f"{s / 3600:.1f}h"


def run_suite(name, cmd):
    """Run a child runner, stream its output live, return (rc, elapsed)."""
    print()
    print("=" * 78)
    print(f"  {name}")
    print(f"  $ {' '.join(cmd)}")
    print("=" * 78)
    t0 = time.time()
    rc = subprocess.call(cmd)
    elapsed = time.time() - t0
    print(f"  → {name}: rc={rc} in {fmt_seconds(elapsed)}")
    return rc, elapsed


def load_summary(path):
    if not os.path.isfile(path):
        return None
    try:
        with open(path) as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return None


def fmt_counts(d):
    if not d:
        return "(no results)"
    return " ".join(f"{k}={v}" for k, v in sorted(d.items()))


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--pipelines",
        default=None,
        help="Comma-separated PIPELINES labels to test (default: all). "
        "Forwarded to the riscv-tests, vector and lockstep runners.",
    )
    ap.add_argument(
        "--skip",
        action="append",
        default=[],
        choices=["riscv-tests", "vector", "multicore", "lockstep"],
        help="Skip a suite (repeat for multiple)",
    )
    ap.add_argument(
        "--smoke",
        action="store_true",
        help="Run each suite's smoke subset",
    )
    return ap.parse_args()


@dataclass
class Suite:
    """A conformance runner, the arguments it is given and where it writes."""

    key: str
    name: str
    script: str
    args: list[str]
    out: str
    takes_pipelines: bool

    def command(self, pipelines):
        cmd = [PYTHON, os.path.join(ROOT, "tests", "conformance", self.script)]
        cmd += ["--out", self.out, *self.args]
        if pipelines and self.takes_pipelines:
            cmd += ["--pipelines", pipelines]
        return cmd


def suites_for(args):
    """The suites `args` asks for, in the order they run. Lockstep has no
    smoke subset, so a smoke run leaves it out."""
    smoke = ["--smoke"] if args.smoke else []
    vector_smoke = ["--smoke", "--build-dir", VECTOR_SMOKE_BUILD] if args.smoke else []
    suites = [
        Suite(
            "riscv-tests",
            "riscv-tests multi-config",
            "riscv_tests.py",
            smoke,
            os.path.join(RESULTS_DIR, "riscv-tests.json"),
            takes_pipelines=True,
        ),
        Suite(
            "vector",
            f"vector multi-config (vlen={args.vlen})",
            "vector_tests.py",
            ["--vlen", str(args.vlen), *vector_smoke],
            os.path.join(RESULTS_DIR, "vector-multi.json"),
            takes_pipelines=True,
        ),
        Suite(
            "multicore",
            "multicore",
            "multicore_tests.py",
            smoke,
            os.path.join(RESULTS_DIR, "multicore.json"),
            takes_pipelines=False,
        ),
        Suite(
            "lockstep",
            "lockstep against spike",
            "lockstep.py",
            [],
            os.path.join(RESULTS_DIR, "lockstep.json"),
            takes_pipelines=True,
        ),
    ]
    return [
        suite
        for suite in suites
        if suite.key not in args.skip and not (args.smoke and suite.key == "lockstep")
    ]


def print_summary(suite_results, total, overall_rc):
    print()
    print("=" * 78)
    print("  UNIFIED SUMMARY")
    print("=" * 78)
    for name, rc, elapsed, summary in suite_results:
        mark = "PASS" if rc == 0 else "FAIL"
        line = f"  [{mark}] {name:50}  {fmt_seconds(elapsed):>8}"
        if summary and "counts" in summary:
            line += f"  {fmt_counts(summary['counts'])}"
        print(line)
    print("=" * 78)
    print(f"  total: {fmt_seconds(total)}, overall rc={overall_rc}")
    print("=" * 78)


def print_failure_breakdown(suite_results):
    """Each suite's failing tests with how many configs each fails on: a
    test failing on every config is a correctness bug, one failing on some
    a particular knob tripping a corner case."""
    any_failures = False
    for name, _rc, _elapsed, summary in suite_results:
        if not summary or "results" not in summary:
            continue
        bad = [r for r in summary["results"] if r["status"] != "pass"]
        if not bad:
            continue
        any_failures = True
        print()
        print(f"  {name} — {len(bad)} non-pass results")
        by_test = {}
        for r in bad:
            by_test.setdefault(r["test"], []).append(r.get("pipeline", "?"))
        total_pipelines = len(summary.get("pipelines") or [None])
        for test in sorted(by_test):
            pipes = by_test[test]
            scope = (
                "ALL configs"
                if len(pipes) == total_pipelines
                else f"{len(pipes)}/{total_pipelines} configs"
            )
            sample = pipes[0] if len(pipes) == 1 else ""
            print(f"    {test:40} {scope:20} {sample}")
    if any_failures:
        print()
        print(
            "  → 'ALL configs' = config-invariant (likely correctness bug or "
            "missing extension)"
        )
        print("  → 'N/M configs' = config-specific (a particular knob trips it)")


def main():
    args = parse_args()
    suites = suites_for(args)
    if not suites:
        sys.exit("nothing to run (everything was skipped)")
    print()
    print("rvsim — full regression sweep")
    print(f"  suites: {[suite.name for suite in suites]}")
    print(f"  results: {RESULTS_DIR}")

    overall_rc = 0
    suite_results = []
    started = time.time()
    for suite in suites:
        rc, elapsed = run_suite(suite.name, suite.command(args.pipelines))
        overall_rc = rc or overall_rc
        suite_results.append((suite.name, rc, elapsed, load_summary(suite.out)))

    print_summary(suite_results, time.time() - started, overall_rc)
    print_failure_breakdown(suite_results)
    sys.exit(overall_rc)


if __name__ == "__main__":
    main()
