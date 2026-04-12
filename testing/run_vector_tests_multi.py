#!/usr/bin/env python3
"""Run the chipsalliance vector cosim suite across every PIPELINES config.

For each test ELF under testing/builds/vector/vlen{N}/:
  1. Compute the spike reference signature ONCE (cached on disk).
  2. For every pipeline config in testing/configs/pipelines.py, run the same
     ELF on rvsim via testing/_worker.py and diff against the cached spike sig.

Subprocess-isolated workers + streaming JSON, same model as the riscof
multi-config runner.

Usage:
    .venv/bin/python testing/run_vector_tests_multi.py
    .venv/bin/python testing/run_vector_tests_multi.py --vlen 256
    .venv/bin/python testing/run_vector_tests_multi.py --pipelines 'inorder w1'
    .venv/bin/python testing/run_vector_tests_multi.py --filter 'vadd'
"""

import argparse
import concurrent.futures as cf
import glob
import json
import os
import struct
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
BUILDS = os.path.join(ROOT, "testing", "builds")
SPIKE = os.path.join(BUILDS, "spike-install", "bin", "spike")
RESULTS_DIR = os.path.join(BUILDS, "results")
TIMEOUT_SEC = 180


def read_sig_file(path):
    out = bytearray()
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                out.extend(struct.pack("<I", int(line, 16)))
    return bytes(out)


def compute_spike_sig(elf_path, vlen, march, sig_path):
    """Run spike once for a test, write its signature to sig_path."""
    isa = f"{march}_zvl{vlen}b" if f"zvl{vlen}b" not in march else march
    res = subprocess.run(
        [
            SPIKE,
            f"--isa={isa}",
            f"+signature={sig_path}",
            "+signature-granularity=4",
            elf_path,
        ],
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SEC,
    )
    if not os.path.isfile(sig_path):
        return f"spike rc={res.returncode}: {res.stderr.strip()[:160]}"
    return None


def cache_spike_sigs(elfs, vlen, march, cache_dir, jobs):
    """Compute spike sigs for every test in parallel, return {elf: sig_path}."""
    os.makedirs(cache_dir, exist_ok=True)
    todo = []
    sig_paths = {}
    for elf in elfs:
        name = os.path.basename(elf)[:-4]
        sig_path = os.path.join(cache_dir, f"{name}.sig")
        sig_paths[elf] = sig_path
        if not os.path.isfile(sig_path):
            todo.append((elf, sig_path))
    if not todo:
        print(f"spike cache: all {len(elfs)} sigs already cached at {cache_dir}")
        return sig_paths

    print(f"spike cache: computing {len(todo)} reference sigs (jobs={jobs})")
    bad = []
    with cf.ProcessPoolExecutor(max_workers=jobs) as ex:
        futs = {
            ex.submit(compute_spike_sig, elf, vlen, march, sig_path): (elf, sig_path)
            for elf, sig_path in todo
        }
        done = 0
        for fut in cf.as_completed(futs):
            done += 1
            elf, sig_path = futs[fut]
            try:
                err = fut.result()
            except Exception as e:
                err = f"{type(e).__name__}: {e}"
            if err:
                bad.append((os.path.basename(elf), err))
            if done % 200 == 0 or done == len(todo):
                print(f"  spike cache: {done}/{len(todo)}", flush=True)
    if bad:
        print(f"WARNING: {len(bad)} tests had no spike signature, will be skipped")
        for n, e in bad[:5]:
            print(f"  {n}: {e}")
    return sig_paths


def run_one(args):
    elf, ref_sig_path, pipeline_label, scratch = args
    name = os.path.basename(elf)[:-4]
    sig_out = os.path.join(scratch, f"{name}.{hash(pipeline_label) & 0xFFFF:04x}.sig")
    t0 = time.time()
    try:
        res = subprocess.run(
            [PYTHON, WORKER, elf, pipeline_label, sig_out],
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SEC,
        )
    except subprocess.TimeoutExpired:
        return dict(test=name, pipeline=pipeline_label, status="timeout")

    if res.returncode == 124:
        try:
            os.remove(sig_out)
        except OSError:
            pass
        return dict(test=name, pipeline=pipeline_label, status="timeout")

    if not os.path.isfile(sig_out):
        msg = (res.stderr or res.stdout).strip().splitlines()
        tail = " | ".join(msg[-3:])[:200]
        return dict(
            test=name, pipeline=pipeline_label, status="error",
            reason=f"rc={res.returncode}: {tail}",
        )

    try:
        with open(sig_out) as f:
            dut_sig = f.read()
        with open(ref_sig_path) as f:
            ref = f.read()
    finally:
        try:
            os.remove(sig_out)
        except OSError:
            pass

    if dut_sig == ref:
        return dict(
            test=name, pipeline=pipeline_label, status="pass",
            seconds=round(time.time() - t0, 2),
        )
    dut_lines = dut_sig.splitlines()
    ref_lines = ref.splitlines()
    diff_at = next(
        (i for i in range(min(len(dut_lines), len(ref_lines)))
         if dut_lines[i] != ref_lines[i]),
        min(len(dut_lines), len(ref_lines)),
    )
    return dict(
        test=name, pipeline=pipeline_label, status="fail",
        diff_line=diff_at,
        seconds=round(time.time() - t0, 2),
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--march",
        default="rv64gcv_zfh_zvfh_zvbb_zvbc_zvkg_zvkned_zvknha_zvksed_zvksh",
    )
    ap.add_argument("--build-dir", default=os.path.join(BUILDS, "vector"))
    ap.add_argument("--filter", default="*")
    ap.add_argument("--pipelines", default=None,
                    help="Comma-separated PIPELINES labels (default: all)")
    ap.add_argument("--smoke", action="store_true",
                    help="single pipeline x first 20 tests")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(RESULTS_DIR, "vector-multi.json"))
    args = ap.parse_args()

    if not os.path.isfile(SPIKE):
        sys.exit(f"ERROR: local spike not found at {SPIKE}\nRun: make vector-test-build")

    pattern = args.filter if args.filter.endswith(".elf") else args.filter + "*.elf"
    elfs = sorted(glob.glob(os.path.join(args.build_dir, f"vlen{args.vlen}", pattern)))
    if not elfs:
        sys.exit(f"no ELFs matching {pattern} under {args.build_dir}/vlen{args.vlen}")

    selected_pipelines = PIPELINES
    if args.pipelines:
        wanted = {p.strip() for p in args.pipelines.split(",")}
        selected_pipelines = [(l, c) for l, c in PIPELINES if l in wanted]
        missing = wanted - {l for l, _ in selected_pipelines}
        if missing:
            sys.exit(f"unknown pipeline label(s): {sorted(missing)}")
    if args.smoke:
        elfs = elfs[:20]
        selected_pipelines = selected_pipelines[:1]

    os.makedirs(RESULTS_DIR, exist_ok=True)
    sig_cache_dir = os.path.join(BUILDS, "vector", f"spike-sigs-vlen{args.vlen}")
    spike_sigs = cache_spike_sigs(elfs, args.vlen, args.march, sig_cache_dir, args.jobs)
    elfs = [e for e in elfs if os.path.isfile(spike_sigs[e])]

    work = [
        (elf, spike_sigs[elf], label, os.path.join(RESULTS_DIR, "scratch_vector"))
        for label, _cfg in selected_pipelines
        for elf in elfs
    ]
    os.makedirs(os.path.join(RESULTS_DIR, "scratch_vector"), exist_ok=True)

    print(
        f"vector multi-config: {len(elfs)} tests x "
        f"{len(selected_pipelines)} pipelines = {len(work)} runs (jobs={args.jobs})"
    )
    print(f"streaming results to: {args.out}")

    results = []
    counts = {}

    def write_partial():
        with open(args.out, "w") as f:
            json.dump(
                {
                    "vlen": args.vlen,
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
                        test=os.path.basename(w[0])[:-4],
                        pipeline=w[2],
                        status="error",
                        reason=f"{type(e).__name__}: {e}",
                    )
                results.append(r)
                counts[r["status"]] = counts.get(r["status"], 0) + 1
                if r["status"] != "pass":
                    extra = ""
                    if r["status"] == "fail":
                        extra = f" diff@{r.get('diff_line', '?')}"
                    elif r["status"] == "error":
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
    print(f"=== vector multi-config (vlen={args.vlen}): {len(results)} / {len(work)} ===")
    for k in ("pass", "fail", "timeout", "skip", "error"):
        if k in counts:
            print(f"  {k:8} {counts[k]}")
    print(f"results: {args.out}")
    failed = sum(counts.get(k, 0) for k in ("fail", "timeout", "error"))
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
