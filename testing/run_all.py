#!/usr/bin/env python3
"""Unified entry point: every test suite × every PIPELINES config.

Runs in order:
  1. testing/run_riscv_tests.py    — riscv-tests across all PIPELINES
  2. testing/run_riscof_tests.py   — riscof arch-test across all PIPELINES
  3. testing/run_vector_tests_multi.py — RVV cosim across all PIPELINES

Each child runner streams its own JSON to testing/builds/results/. This
script tails their stdout, captures pass/fail counts from those JSONs at
the end, prints a unified summary, and exits non-zero if any suite failed.

This is the headline "did I break anything?" command. Expect several CPU-
hours on a workstation; an SSD and a lot of cores help.

Usage:
    .venv/bin/python testing/run_all.py
    .venv/bin/python testing/run_all.py --skip vector
    .venv/bin/python testing/run_all.py --vlen 256
    .venv/bin/python testing/run_all.py --pipelines 'inorder w1,o3 w4'
"""

import argparse
import json
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PYTHON = os.path.join(ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
RESULTS_DIR = os.path.join(ROOT, "testing", "builds", "results")


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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--pipelines",
        default=None,
        help="Comma-separated PIPELINES labels to test (default: all). "
        "Forwarded to riscof and vector multi-config runners.",
    )
    ap.add_argument(
        "--skip",
        action="append",
        default=[],
        choices=["riscv-tests", "riscof", "vector"],
        help="Skip a suite (repeat for multiple)",
    )
    ap.add_argument(
        "--smoke",
        action="store_true",
        help="Pass --smoke to each child runner",
    )
    args = ap.parse_args()

    suites = []

    if "riscv-tests" not in args.skip:
        out = os.path.join(RESULTS_DIR, "riscv-tests.json")
        cmd = [PYTHON, os.path.join(ROOT, "testing/run_riscv_tests.py"),
               "--out", out]
        if args.pipelines:
            cmd += ["--pipelines", args.pipelines]
        if args.smoke:
            cmd += ["--smoke"]
        suites.append(("riscv-tests multi-config", cmd, out))

    if "riscof" not in args.skip:
        out = os.path.join(RESULTS_DIR, "riscof-multi.json")
        cmd = [PYTHON, os.path.join(ROOT, "testing/run_riscof_tests.py"),
               "--out", out]
        if args.pipelines:
            cmd += ["--pipelines", args.pipelines]
        if args.smoke:
            cmd += ["--smoke"]
        suites.append(("riscof multi-config", cmd, out))

    if "vector" not in args.skip:
        out = os.path.join(RESULTS_DIR, "vector-multi.json")
        cmd = [PYTHON, os.path.join(ROOT, "testing/run_vector_tests_multi.py"),
               "--vlen", str(args.vlen), "--out", out]
        if args.pipelines:
            cmd += ["--pipelines", args.pipelines]
        if args.smoke:
            cmd += ["--smoke"]
        suites.append((f"vector multi-config (vlen={args.vlen})", cmd, out))

    if not suites:
        sys.exit("nothing to run (everything was skipped)")

    print()
    print("rvsim — full regression sweep")
    print(f"  suites: {[s[0] for s in suites]}")
    print(f"  results: {RESULTS_DIR}")

    overall_rc = 0
    suite_results = []
    t_overall = time.time()
    for name, cmd, out in suites:
        rc, elapsed = run_suite(name, cmd)
        if rc != 0:
            overall_rc = rc
        summary = load_summary(out) if out else None
        suite_results.append((name, rc, elapsed, summary))

    total = time.time() - t_overall

    # ── Final unified summary ────────────────────────────────────────────────
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

    # ── Failure breakdown ────────────────────────────────────────────────────
    # The point of running every test on every pipeline config is to learn
    # which failures are config-invariant (real bugs in correctness logic) vs.
    # config-specific (a particular pipeline knob trips a corner case). Group
    # failures by test name and show the pipeline count.
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
        # Group by test name → set of pipelines
        by_test = {}
        for r in bad:
            key = r.get("test", r.get("name", "?"))
            by_test.setdefault(key, []).append(r.get("pipeline", "?"))
        total_pipelines = len(summary.get("pipelines") or [None])
        for test in sorted(by_test):
            pipes = by_test[test]
            scope = (
                "ALL configs" if len(pipes) == total_pipelines
                else f"{len(pipes)}/{total_pipelines} configs"
            )
            sample = pipes[0] if len(pipes) == 1 else ""
            print(f"    {test:40} {scope:20} {sample}")
    if any_failures:
        print()
        print("  → 'ALL configs' = config-invariant (likely correctness bug or "
              "missing extension)")
        print("  → 'N/M configs' = config-specific (a particular knob trips it)")
    sys.exit(overall_rc)


if __name__ == "__main__":
    main()
