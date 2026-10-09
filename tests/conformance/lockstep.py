#!/usr/bin/env python3
"""Run programs on rvsim in lockstep with spike, on every PIPELINES config.

rvsim writes its commit log into a FIFO that tools/lockstep/spike_lockstep
replays on spike one instruction at a time, stopping at the first
instruction whose PC, privilege mode, destination register, CSR write or
memory access differs. The workloads are the riscv-tests -p- suites, the
bundled programs, the benchmarks and the RVV tests (the sample
`make vector-smoke-build` builds with --smoke, else the full set
`make vector-test-build` builds); multi-hart programs are left out, as the
driver models one hart.

Usage:
    .venv/bin/python tests/conformance/lockstep.py
    .venv/bin/python tests/conformance/lockstep.py --smoke
    .venv/bin/python tests/conformance/lockstep.py --pipelines 'inorder w1,o3 w4' --filter fib
"""

import argparse
import concurrent.futures as cf
import glob
import json
import os
import re
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from tests.conformance.configs.pipelines import PIPELINES, SMOKE_PIPELINES
from tests.conformance.riscv_tests import find_tests as find_riscv_tests

WORKER = os.path.join(ROOT, "tests", "conformance", "_worker.py")
DRIVER = os.path.join(ROOT, "tests", "builds", "lockstep", "spike_lockstep")
PYTHON = os.path.join(ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
SOFTWARE_BIN = os.path.join(ROOT, "software", "bin")
VECTOR_BUILD = os.path.join(ROOT, "tests", "builds", "vector")
VECTOR_SMOKE_BUILD = os.path.join(ROOT, "tests", "builds", "vector-smoke")
RESULTS_DIR = os.path.join(ROOT, "tests", "builds", "results")
TIMEOUT_SEC = 600

# The multi-letter extensions rvsim implements (docs/architecture/isa.md);
# the driver keeps those whose base extension the hart's misa has.
RVSIM_EXTENSIONS = [
    "zicsr",
    "zifencei",
    "zicntr",
    "zba",
    "zbb",
    "zbc",
    "zbs",
    "zbkb",
    "zbkx",
    "zfh",
    "zicbom",
    "zicboz",
    "zvfh",
    "zvbb",
    "zvbc",
    "zvkg",
    "zvkned",
    "zvknhb",
    "zvksed",
    "zvksh",
    "sstc",
]


def vlen_for(elf_path, cfg):
    """The VLEN `elf_path` runs at: an RVV test's, built for one VLEN and
    found under `vlen<N>/` (the worker pins it the same way), else `cfg`'s."""
    pinned = re.search(r"/vlen(\d+)/", elf_path)
    return int(pinned.group(1)) if pinned else cfg.vlen


def spike_extensions(cfg, vlen):
    """Spike's multi-letter extensions for a hart configured as `cfg` at `vlen`."""
    extensions = list(RVSIM_EXTENSIONS)
    extensions.append(f"zvl{vlen}b")
    if not cfg.misaligned_access_trap:
        extensions.append("zicclsm")
    if cfg.svadu:
        extensions.append("svadu")
    return "_".join(extensions)


def find_workloads(filter_substr=None, smoke=False):
    """(name, path) of every single-hart workload."""
    workloads = list(find_riscv_tests())
    for kind in ("programs", "benchmarks"):
        for path in sorted(glob.glob(os.path.join(SOFTWARE_BIN, kind, "*.elf"))):
            workloads.append((f"{kind}/{os.path.basename(path)}", path))
    vector_build = VECTOR_SMOKE_BUILD if smoke else VECTOR_BUILD
    for path in sorted(glob.glob(os.path.join(vector_build, "vlen*", "*.elf"))):
        workloads.append((f"vector/{os.path.basename(path)}", path))
    if filter_substr:
        workloads = [(name, path) for name, path in workloads if filter_substr in name]
    return workloads


def lockstep_one(args):
    """Runs one workload on one pipeline in lockstep and returns its result."""
    name, elf_path, label, cycle_limit = args
    cfg = dict(PIPELINES)[label]
    result = {"test": name, "pipeline": label}
    started = time.time()
    with tempfile.TemporaryDirectory(prefix="lockstep-") as tmp:
        fifo = os.path.join(tmp, "commit.log")
        os.mkfifo(fifo)
        driver_cmd = [
            DRIVER,
            "--log",
            fifo,
            "--extensions",
            spike_extensions(cfg, vlen_for(elf_path, cfg)),
            "--elf",
            elf_path,
            "--ram",
            f"{cfg.ram_base:x}:{cfg.ram_size:x}",
            "--mmu",
            cfg.paging_mode_max,
        ]
        sim_env = dict(os.environ, RVSIM_CYCLE_LIMIT=str(cycle_limit))
        with open(os.path.join(tmp, "sim.err"), "w+") as sim_err:
            driver = subprocess.Popen(
                driver_cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
            )
            sim = subprocess.Popen(
                [PYTHON, WORKER, elf_path, label, "--commit-log", fifo],
                stdout=subprocess.DEVNULL,
                stderr=sim_err,
                env=sim_env,
            )
            try:
                report, _ = driver.communicate(timeout=TIMEOUT_SEC)
            except subprocess.TimeoutExpired:
                driver.kill()
                sim.kill()
                driver.wait()
                sim.wait()
                return {**result, "status": "timeout"}
            if driver.returncode != 0:
                sim.kill()
            sim_rc = sim.wait()
            sim_err.seek(0)
            sim_error = sim_err.read().strip().splitlines()

    result["seconds"] = round(time.time() - started, 2)
    lines = report.strip().splitlines()
    if driver.returncode == 0 and sim_rc in (0, 1, 124):
        return {**result, "status": "pass", "summary": lines[-1] if lines else ""}
    if driver.returncode == 1:
        return {**result, "status": "diverged", "reason": lines[0], "detail": lines[1:]}
    reason = " | ".join(lines[-2:] + sim_error[-2:])[:300]
    return {
        **result,
        "status": "error",
        "rc": driver.returncode,
        "sim_rc": sim_rc,
        "reason": reason,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--pipelines", default=None, help="Comma-separated PIPELINES labels"
    )
    ap.add_argument("--filter", default=None, help="Substring filter on workload name")
    ap.add_argument(
        "--smoke", action="store_true", help="every workload on SMOKE_PIPELINES"
    )
    ap.add_argument("--cycle-limit", type=int, default=2_000_000)
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(RESULTS_DIR, "lockstep.json"))
    args = ap.parse_args()

    if not os.path.isfile(DRIVER):
        sys.exit(f"ERROR: {DRIVER} not found\nRun: make lockstep-build")
    workloads = find_workloads(args.filter, args.smoke)
    if not workloads:
        sys.exit("ERROR: no workloads found\nRun: make riscv-tests-build software")

    pipelines = PIPELINES
    if args.pipelines:
        wanted = {p.strip() for p in args.pipelines.split(",")}
        pipelines = [(label, c) for label, c in PIPELINES if label in wanted]
        missing = wanted - {label for label, _ in pipelines}
        if missing:
            sys.exit(f"unknown pipeline label(s): {sorted(missing)}")
    if args.smoke:
        pipelines = [(label, c) for label, c in PIPELINES if label in SMOKE_PIPELINES]
    pipelines = [(label, c) for label, c in pipelines if c.hart_count == 1]

    work = [
        (name, path, label, args.cycle_limit)
        for label, _ in pipelines
        for name, path in workloads
    ]
    print(
        f"lockstep: {len(workloads)} workloads x {len(pipelines)} pipelines "
        f"= {len(work)} runs (jobs={args.jobs})"
    )

    results = []
    counts = {}
    os.makedirs(RESULTS_DIR, exist_ok=True)
    try:
        with cf.ProcessPoolExecutor(max_workers=args.jobs) as ex:
            futures = {ex.submit(lockstep_one, w): w for w in work}
            for i, future in enumerate(cf.as_completed(futures), 1):
                try:
                    r = future.result()
                except Exception as e:
                    name, _, label, _ = futures[future]
                    r = {
                        "test": name,
                        "pipeline": label,
                        "status": "error",
                        "reason": f"{type(e).__name__}: {e}",
                    }
                results.append(r)
                counts[r["status"]] = counts.get(r["status"], 0) + 1
                if r["status"] != "pass":
                    print(
                        f"[{i:6}/{len(work)}] {r['status'].upper():8} {r['pipeline']:24} "
                        f"{r['test']}: {r.get('reason', '')}",
                        flush=True,
                    )
    finally:
        with open(args.out, "w") as f:
            json.dump(
                {
                    "counts": counts,
                    "pipelines": [label for label, _ in pipelines],
                    "results": results,
                },
                f,
                indent=2,
            )

    print()
    print(f"=== lockstep: {len(results)} / {len(work)} completed ===")
    for status in ("pass", "diverged", "timeout", "error"):
        if status in counts:
            print(f"  {status:9} {counts[status]}")
    print(f"results: {args.out}")
    sys.exit(0 if counts.get("pass", 0) == len(work) else 1)


if __name__ == "__main__":
    main()
