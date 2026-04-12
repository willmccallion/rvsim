#!/usr/bin/env python3
"""Run riscv-tests ISA suite across every PIPELINES config.

Each test runs in its own subprocess (testing/_worker.py) so a panic /
segfault in rvsim only kills one test, never the whole sweep. Results stream
to disk every 50 tests.

The shared PIPELINES matrix lives in testing/configs/pipelines.py — edit it
once, all three runners pick up the change.

Usage:
    .venv/bin/python testing/run_riscv_tests.py
    .venv/bin/python testing/run_riscv_tests.py --pipelines 'inorder w1,o3 w4'
    .venv/bin/python testing/run_riscv_tests.py --filter rv64ui
"""

import argparse
import concurrent.futures as cf
import glob
import json
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from testing.configs.pipelines import PIPELINES  # noqa: E402

WORKER = os.path.join(ROOT, "testing", "_worker.py")
PYTHON = os.path.join(ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
ISA_DIR = os.path.join(ROOT, "testing", "builds", "riscv-tests", "isa")
RESULTS_DIR = os.path.join(ROOT, "testing", "builds", "results")
TIMEOUT_SEC = 60

# Test suites we care about: physical-mode (-p-) variants only.
SUITES = [
    "rv64ui", "rv64um", "rv64ua", "rv64uf", "rv64ud", "rv64uc",
    "rv64mi", "rv64si",
]


def find_tests(filter_substr=None):
    tests = []
    for suite in SUITES:
        for path in sorted(glob.glob(os.path.join(ISA_DIR, f"{suite}-p-*"))):
            if path.endswith(".dump"):
                continue
            name = os.path.basename(path)
            if filter_substr and filter_substr not in name:
                continue
            tests.append((name, path))
    return tests


def run_one(args):
    name, elf_path, pipeline_label = args
    t0 = time.time()
    try:
        res = subprocess.run(
            [PYTHON, WORKER, elf_path, pipeline_label],
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SEC,
        )
    except subprocess.TimeoutExpired:
        return dict(test=name, pipeline=pipeline_label, status="timeout")
    elapsed = round(time.time() - t0, 2)

    if res.returncode == 0:
        return dict(test=name, pipeline=pipeline_label, status="pass", seconds=elapsed)
    if res.returncode == 124:
        return dict(test=name, pipeline=pipeline_label, status="timeout", seconds=elapsed)
    msg = (res.stderr or res.stdout).strip().splitlines()
    tail = " | ".join(msg[-3:])[:200]
    return dict(
        test=name,
        pipeline=pipeline_label,
        status="fail" if res.returncode == 1 else "error",
        rc=res.returncode,
        reason=tail,
        seconds=elapsed,
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pipelines", default=None,
                    help="Comma-separated PIPELINES labels (default: all)")
    ap.add_argument("--filter", default=None,
                    help="Substring filter on test name (e.g. rv64ui)")
    ap.add_argument("--smoke", action="store_true",
                    help="single pipeline x first 50 tests")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(RESULTS_DIR, "riscv-tests.json"))
    args = ap.parse_args()

    tests = find_tests(args.filter)
    if not tests:
        sys.exit(
            f"ERROR: no tests found in {ISA_DIR}\n"
            f"Run: make riscv-tests-build"
        )

    selected_pipelines = PIPELINES
    if args.pipelines:
        wanted = {p.strip() for p in args.pipelines.split(",")}
        selected_pipelines = [(l, c) for l, c in PIPELINES if l in wanted]
        missing = wanted - {l for l, _ in selected_pipelines}
        if missing:
            sys.exit(f"unknown pipeline label(s): {sorted(missing)}")
    if args.smoke:
        selected_pipelines = selected_pipelines[:1]
        tests = tests[:50]

    work = [
        (name, path, label)
        for label, _cfg in selected_pipelines
        for name, path in tests
    ]
    os.makedirs(RESULTS_DIR, exist_ok=True)

    print(
        f"riscv-tests: {len(tests)} tests x {len(selected_pipelines)} pipelines "
        f"= {len(work)} runs (jobs={args.jobs})"
    )
    print(f"streaming results to: {args.out}")

    results = []
    counts = {}

    def write_partial():
        with open(args.out, "w") as f:
            json.dump(
                {
                    "total_planned": len(work),
                    "completed": len(results),
                    "counts": counts,
                    "pipelines": [l for l, _ in selected_pipelines],
                    "results": results,
                },
                f,
                indent=2,
            )

    try:
        with cf.ProcessPoolExecutor(max_workers=args.jobs) as ex:
            futs = {ex.submit(run_one, w): w for w in work}
            for i, fut in enumerate(cf.as_completed(futs), 1):
                try:
                    r = fut.result()
                except Exception as e:
                    w = futs[fut]
                    r = dict(
                        test=w[0], pipeline=w[2], status="error",
                        reason=f"{type(e).__name__}: {e}",
                    )
                results.append(r)
                counts[r["status"]] = counts.get(r["status"], 0) + 1
                if r["status"] != "pass":
                    extra = ""
                    if r["status"] in ("fail", "error"):
                        extra = f" ({(r.get('reason') or '')[:60]})"
                    print(
                        f"[{i:6}/{len(work)}] {r['status'].upper():7} "
                        f"{r['pipeline']:24} {r['test']}{extra}",
                        flush=True,
                    )
                if i % 200 == 0:
                    write_partial()
                    pct = 100 * i / len(work)
                    print(
                        f"  ... {i}/{len(work)} ({pct:.1f}%) — "
                        + " ".join(f"{k}={v}" for k, v in sorted(counts.items())),
                        flush=True,
                    )
    finally:
        write_partial()

    print()
    print(f"=== riscv-tests: {len(results)} / {len(work)} completed ===")
    for k in ("pass", "fail", "timeout", "error"):
        if k in counts:
            print(f"  {k:8} {counts[k]}")
    print(f"results: {args.out}")
    failed = sum(counts.get(k, 0) for k in ("fail", "timeout", "error"))
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
