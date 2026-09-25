#!/usr/bin/env python3
"""Run the multi-hart programs under software/bin/multicore on 2 and 4 harts.

Each program is self-checking: hart 0 exits 0 when the cross-hart result
is exact and 1 otherwise. Every program runs on every selected PIPELINES
config through testing/_worker.py, one subprocess per run.

Usage:
    .venv/bin/python testing/run_multicore_tests.py
    .venv/bin/python testing/run_multicore_tests.py --pipelines 'inorder w1,o3 w4'
    .venv/bin/python testing/run_multicore_tests.py --harts 2,4,8
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
ELF_DIR = os.path.join(ROOT, "software", "bin", "multicore")
RESULTS_DIR = os.path.join(ROOT, "testing", "builds", "results")
TIMEOUT_SEC = 300

DEFAULT_PIPELINES = [
    "inorder w1",
    "inorder w4",
    "o3 w1",
    "o3 w4",
    "o3 w4 small-rob",
    "o3 w4 mshr-1",
    "o3 w4 no-l2",
    "o3 w4 l3",
    "o3 w4 ddr5",
    "ref p550",
]


def run_one(args):
    name, elf_path, label, harts = args
    env = dict(os.environ, RVSIM_HART_COUNT=str(harts))
    t0 = time.time()
    try:
        res = subprocess.run(
            [PYTHON, WORKER, elf_path, label],
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SEC,
            env=env,
        )
    except subprocess.TimeoutExpired:
        return dict(test=name, pipeline=label, harts=harts, status="timeout")
    elapsed = round(time.time() - t0, 2)
    status = {0: "pass", 1: "fail", 124: "timeout"}.get(res.returncode, "error")
    tail = " | ".join((res.stdout + res.stderr).strip().splitlines()[-3:])[:300]
    return dict(test=name, pipeline=label, harts=harts, status=status, rc=res.returncode,
                output=tail, seconds=elapsed)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pipelines", default=None, help="Comma-separated PIPELINES labels")
    ap.add_argument("--harts", default="2,4", help="Comma-separated hart counts")
    ap.add_argument("--filter", default=None, help="Substring filter on program name")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(RESULTS_DIR, "multicore.json"))
    args = ap.parse_args()

    elfs = sorted(glob.glob(os.path.join(ELF_DIR, "*.elf")))
    if args.filter:
        elfs = [e for e in elfs if args.filter in os.path.basename(e)]
    if not elfs:
        sys.exit(f"ERROR: no programs in {ELF_DIR}; run: make software")

    labels = args.pipelines.split(",") if args.pipelines else DEFAULT_PIPELINES
    known = {lbl for lbl, _ in PIPELINES}
    unknown = [l for l in labels if l.strip() not in known]
    if unknown:
        sys.exit(f"unknown pipeline label(s): {unknown}")
    harts = [int(h) for h in args.harts.split(",")]

    work = [
        (os.path.basename(elf), elf, label.strip(), n)
        for elf in elfs
        for label in labels
        for n in harts
    ]
    results = []
    failed = 0
    with cf.ProcessPoolExecutor(max_workers=args.jobs) as pool:
        for r in pool.map(run_one, work):
            results.append(r)
            mark = "ok  " if r["status"] == "pass" else "FAIL"
            if r["status"] != "pass":
                failed += 1
            print(f"{mark} {r['test']:24s} {r['pipeline']:22s} harts={r['harts']} "
                  f"{r.get('output', '')}", flush=True)

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(results, f, indent=1)
    print(f"{len(results) - failed}/{len(results)} passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
