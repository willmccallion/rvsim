#!/usr/bin/env python3
"""Run the riscof arch-test suite across every PIPELINES config.

The riscof harness at testing/riscof/ already compiles every test once into
testing/builds/riscof-work/.../<test>.S/{dut/dut.elf, ref/Reference-spike.signature}.
This runner walks that tree, then for each pipeline config in
testing/configs/pipelines.py runs every dut.elf on rvsim and diffs its
post-execution signature against the existing spike reference signature.

Spike doesn't change between pipeline configs (it has none), so we reuse the
reference signature for free — only rvsim re-executes per config.

Each test runs in its own subprocess (testing/_worker.py) so a panic or
segfault in rvsim only kills that one test, never the whole sweep. Results
stream to disk every 50 tests.

Usage:
    .venv/bin/python testing/run_riscof_tests.py
    .venv/bin/python testing/run_riscof_tests.py --pipelines 'inorder w1,o3 w4'
    .venv/bin/python testing/run_riscof_tests.py --filter 'rv64i_m/I/'
"""

import argparse
import concurrent.futures as cf
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
WORK_DIR = os.path.join(ROOT, "testing", "builds", "riscof-work")
RESULTS_DIR = os.path.join(ROOT, "testing", "builds", "results")
TIMEOUT_SEC = 120


def find_tests(filter_substr=None):
    """Walk riscof_work and yield (test_name, dut_elf, ref_sig) tuples.

    Each riscof test lives at <ext_group>/<ext>/src/<test>.S/{dut, ref}/.
    """
    tests = []
    for ext_group in sorted(os.listdir(WORK_DIR)):
        ext_group_dir = os.path.join(WORK_DIR, ext_group)
        if not os.path.isdir(ext_group_dir):
            continue
        for ext in sorted(os.listdir(ext_group_dir)):
            src_dir = os.path.join(ext_group_dir, ext, "src")
            if not os.path.isdir(src_dir):
                continue
            for test_s in sorted(os.listdir(src_dir)):
                test_dir = os.path.join(src_dir, test_s)
                dut_elf = os.path.join(test_dir, "dut", "dut.elf")
                ref_sig = os.path.join(test_dir, "ref", "Reference-spike.signature")
                if not (os.path.isfile(dut_elf) and os.path.isfile(ref_sig)):
                    continue
                rel = os.path.relpath(test_dir, WORK_DIR)
                if filter_substr and filter_substr not in rel:
                    continue
                tests.append((rel, dut_elf, ref_sig))
    return tests


def run_one(args):
    rel, dut_elf, ref_sig, pipeline_label, scratch = args
    safe_name = rel.replace("/", "_")
    sig_out = os.path.join(scratch, f"{safe_name}.{hash(pipeline_label) & 0xFFFF:04x}.sig")
    t0 = time.time()
    try:
        res = subprocess.run(
            [PYTHON, WORKER, dut_elf, pipeline_label, sig_out],
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SEC,
        )
    except subprocess.TimeoutExpired:
        return dict(test=rel, pipeline=pipeline_label, status="timeout")
    finally:
        pass

    if res.returncode == 124:
        try:
            os.remove(sig_out)
        except OSError:
            pass
        return dict(test=rel, pipeline=pipeline_label, status="timeout")

    if not os.path.isfile(sig_out):
        # Worker crashed/panicked before writing sig
        msg = (res.stderr or res.stdout).strip().splitlines()
        tail = " | ".join(msg[-3:])[:200]
        return dict(
            test=rel,
            pipeline=pipeline_label,
            status="error",
            reason=f"rc={res.returncode}: {tail}",
        )

    try:
        with open(sig_out) as f:
            dut_sig = f.read()
        with open(ref_sig) as f:
            ref = f.read()
    finally:
        try:
            os.remove(sig_out)
        except OSError:
            pass

    if dut_sig == ref:
        return dict(
            test=rel, pipeline=pipeline_label, status="pass",
            seconds=round(time.time() - t0, 2),
        )
    # Find first differing line for the report
    dut_lines = dut_sig.splitlines()
    ref_lines = ref.splitlines()
    diff_at = next(
        (i for i in range(min(len(dut_lines), len(ref_lines)))
         if dut_lines[i] != ref_lines[i]),
        min(len(dut_lines), len(ref_lines)),
    )
    return dict(
        test=rel, pipeline=pipeline_label, status="fail",
        diff_line=diff_at,
        seconds=round(time.time() - t0, 2),
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--pipelines",
        default=None,
        help="Comma-separated PIPELINES labels (default: all)",
    )
    ap.add_argument("--filter", default=None, help="Substring filter on test path")
    ap.add_argument("--smoke", action="store_true", help="Single pipeline x first 50 tests")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument(
        "--out",
        default=os.path.join(RESULTS_DIR, "riscof-multi.json"),
    )
    args = ap.parse_args()

    if not os.path.isdir(WORK_DIR):
        sys.exit(
            f"ERROR: {WORK_DIR} not found.\n"
            f"Run: make arch-test  (populates riscof-work)"
        )

    tests = find_tests(args.filter)
    if not tests:
        sys.exit(f"ERROR: no tests found under {WORK_DIR}")

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

    os.makedirs(RESULTS_DIR, exist_ok=True)
    scratch = os.path.join(RESULTS_DIR, "scratch_riscof")
    os.makedirs(scratch, exist_ok=True)

    total = len(tests) * len(selected_pipelines)
    print(
        f"riscof multi-config: {len(tests)} tests x "
        f"{len(selected_pipelines)} pipelines = {total} runs (jobs={args.jobs})"
    )
    print(f"streaming results to: {args.out}")

    work = [
        (rel, dut, ref, label, scratch)
        for label, _cfg in selected_pipelines
        for rel, dut, ref in tests
    ]

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
                        test=w[0], pipeline=w[3], status="error",
                        reason=f"{type(e).__name__}: {e}",
                    )
                results.append(r)
                counts[r["status"]] = counts.get(r["status"], 0) + 1
                if r["status"] != "pass":
                    extra = ""
                    if r["status"] == "fail":
                        extra = f" diff@{r.get('diff_line', '?')}"
                    elif r["status"] in ("error",):
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
    print(f"=== riscof multi-config: {len(results)} / {len(work)} completed ===")
    for k in ("pass", "fail", "timeout", "error"):
        if k in counts:
            print(f"  {k:8} {counts[k]}")
    print(f"results: {args.out}")
    failed = sum(counts.get(k, 0) for k in ("fail", "timeout", "error"))
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
