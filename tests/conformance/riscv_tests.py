#!/usr/bin/env python3
"""Run riscv-tests ISA suite across every PIPELINES config.

Each test runs in its own subprocess (tests/conformance/_worker.py) so a panic /
segfault in rvsim only kills one test, never the whole sweep. Results stream
to disk every 50 tests.

The shared PIPELINES matrix lives in tests/conformance/configs/pipelines.py — edit it
once, all three runners pick up the change.

Usage:
    .venv/bin/python tests/conformance/riscv_tests.py
    .venv/bin/python tests/conformance/riscv_tests.py --pipelines 'inorder w1,o3 w4'
    .venv/bin/python tests/conformance/riscv_tests.py --filter rv64ui
"""

import argparse
import glob
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from tests.conformance.common import Sweep, run_sweep, select_pipelines
from tests.conformance.configs.pipelines import PIPELINES, SMOKE_PIPELINES

WORKER = os.path.join(ROOT, "tests", "conformance", "_worker.py")
PYTHON = os.path.join(ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
ISA_DIR = os.path.join(ROOT, "tests", "builds", "riscv-tests", "isa")
RESULTS_DIR = os.path.join(ROOT, "tests", "builds", "results")
TIMEOUT_SEC = 60

# Test suites we care about: physical-mode (-p-) variants only.
SUITES = [
    "rv64ui",
    "rv64um",
    "rv64ua",
    "rv64uf",
    "rv64ud",
    "rv64uc",
    "rv64uzba",
    "rv64uzbb",
    "rv64uzbc",
    "rv64uzbs",
    "rv64uzfh",
    "rv64mi",
    "rv64si",
]


def not_applicable(test_name, config):
    """Why `test_name` cannot pass on `config` by the ISA's rules, or None.

    `ma_data` checks that misaligned loads and stores complete with the
    right value, which holds only where the hardware performs them or a
    trap handler emulates them; the bare -p environment has no emulator.
    A config that raises address-misaligned exceptions is checked by
    `ma_addr` instead.
    """
    if "-ma_data" in test_name and config.misaligned_access_trap:
        return "needs misaligned accesses; this config traps them (see ma_addr)"
    return None


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
            check=False,
        )
    except subprocess.TimeoutExpired:
        return {"test": name, "pipeline": pipeline_label, "status": "timeout"}
    elapsed = round(time.time() - t0, 2)

    if res.returncode == 0:
        return {
            "test": name,
            "pipeline": pipeline_label,
            "status": "pass",
            "seconds": elapsed,
        }
    if res.returncode == 124:
        return {
            "test": name,
            "pipeline": pipeline_label,
            "status": "timeout",
            "seconds": elapsed,
        }
    msg = (res.stderr or res.stdout).strip().splitlines()
    tail = " | ".join(msg[-3:])[:200]
    return {
        "test": name,
        "pipeline": pipeline_label,
        "status": "fail" if res.returncode == 1 else "error",
        "rc": res.returncode,
        "reason": tail,
        "seconds": elapsed,
    }


def parse_args():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--pipelines",
        default=None,
        help="Comma-separated PIPELINES labels (default: all)",
    )
    ap.add_argument(
        "--filter", default=None, help="Substring filter on test name (e.g. rv64ui)"
    )
    ap.add_argument(
        "--smoke", action="store_true", help="every test on SMOKE_PIPELINES only"
    )
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument(
        "--hart-count",
        type=int,
        default=None,
        help="Run every test on a system with this many harts",
    )
    ap.add_argument("--out", default=os.path.join(RESULTS_DIR, "riscv-tests.json"))
    return ap.parse_args()


def plan(tests, pipelines):
    """The runs of `tests` on `pipelines`, and the skip results of those a
    pipeline cannot run."""
    work, skipped = [], []
    for label, cfg in pipelines:
        for name, path in tests:
            reason = not_applicable(name, cfg)
            if reason is None:
                work.append((name, path, label))
            else:
                skipped.append(
                    {
                        "test": name,
                        "pipeline": label,
                        "status": "skip",
                        "reason": reason,
                    }
                )
    return work, skipped


def main():
    args = parse_args()
    if args.hart_count:
        os.environ["RVSIM_HART_COUNT"] = str(args.hart_count)
    tests = find_tests(args.filter)
    if not tests:
        sys.exit(f"ERROR: no tests found in {ISA_DIR}\nRun: make riscv-tests-build")
    pipelines = select_pipelines(
        PIPELINES, args.pipelines, SMOKE_PIPELINES if args.smoke else None
    )
    work, skipped = plan(tests, pipelines)
    print(
        f"riscv-tests: {len(tests)} tests x {len(pipelines)} pipelines "
        f"= {len(work)} runs (jobs={args.jobs}), {len(skipped)} not applicable"
    )
    for r in skipped:
        print(f"  SKIP    {r['pipeline']:24} {r['test']} ({r['reason']})")
    sweep = Sweep(
        title="riscv-tests",
        run_one=run_one,
        work=work,
        out=args.out,
        describe=lambda run: (run[0], run[2]),
        jobs=args.jobs,
        header={"pipelines": [label for label, _ in pipelines]},
        skipped=skipped,
    )
    sys.exit(run_sweep(sweep))


if __name__ == "__main__":
    main()
