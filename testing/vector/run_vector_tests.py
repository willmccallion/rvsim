#!/usr/bin/env python3
"""Run RVV tests on rvsim and diff against spike (cosim).

For each ELF under build/vlen{VLEN}/, this script:
  1. Runs it on a local spike (built from source) with +signature.
  2. Runs it on rvsim (in an isolated subprocess) and dumps the same region.
  3. Compares the two — pass iff identical.

Both runs happen in subprocesses so a panic / segfault in either simulator
kills only one test, never the whole sweep. Results are streamed to disk as
each test finishes so a SIGINT or crash mid-sweep still leaves a partial
results.json behind for triage.

Usage:
    python run_vector_tests.py [--vlen 128] [--filter 'vadd*'] [--smoke]
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

HERE = os.path.dirname(os.path.abspath(__file__))
TESTING = os.path.dirname(HERE)
REPO_ROOT = os.path.dirname(TESTING)
BUILDS = os.path.join(TESTING, "builds")
SPIKE = os.path.join(BUILDS, "spike-install", "bin", "spike")
RVSIM_WORKER = os.path.join(TESTING, "riscof", "rvsim", "rvsim_run.py")
PYTHON = os.path.join(REPO_ROOT, ".venv", "bin", "python3")
if not os.path.isfile(PYTHON):
    PYTHON = sys.executable
READELF = "riscv64-elf-readelf"
TIMEOUT_SEC = 120


def get_signature_range(elf_path):
    out = subprocess.run(
        [READELF, "-s", elf_path], capture_output=True, text=True, check=True
    ).stdout
    begin = end = None
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 8:
            sym = parts[-1]
            if sym == "begin_signature":
                begin = int(parts[1], 16)
            elif sym == "end_signature":
                end = int(parts[1], 16)
    return begin, end


def read_sig_file(path):
    """Read a +signature-style hex-words-per-line file → bytes."""
    out = bytearray()
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                out.extend(struct.pack("<I", int(line, 16)))
    return bytes(out)


def run_spike(elf_path, vlen, march, sig_path):
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
        return None, f"spike no sig (rc={res.returncode}): {res.stderr.strip()[:160]}"
    return read_sig_file(sig_path), None


def run_rvsim(elf_path, sig_path):
    """Spawn the existing rvsim_run.py worker in an isolated subprocess.

    Returns (sig_bytes, error_message). Crashes/panics surface as a non-zero
    exit code and an error string from stderr.
    """
    res = subprocess.run(
        [PYTHON, RVSIM_WORKER, elf_path, sig_path],
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SEC,
    )
    if res.returncode != 0:
        msg = (res.stderr or res.stdout).strip().splitlines()
        tail = " | ".join(msg[-3:])[:200] if msg else ""
        return None, f"rvsim rc={res.returncode}: {tail}"
    if not os.path.isfile(sig_path):
        return None, "rvsim no sig"
    return read_sig_file(sig_path), None


def run_one(args):
    elf_path, vlen, march, scratch = args
    name = os.path.basename(elf_path)[:-4]
    spike_sig_path = os.path.join(scratch, f"{name}.spike.sig")
    rvsim_sig_path = os.path.join(scratch, f"{name}.rvsim.sig")
    t0 = time.time()
    try:
        spike_sig, spike_err = run_spike(elf_path, vlen, march, spike_sig_path)
        if spike_sig is None:
            return dict(name=name, status="skip", reason=spike_err)

        rvsim_sig, rvsim_err = run_rvsim(elf_path, rvsim_sig_path)
        if rvsim_sig is None:
            # rvsim crashed/panicked — count as fail with the error captured
            return dict(
                name=name,
                status="error",
                reason=rvsim_err,
                seconds=round(time.time() - t0, 2),
            )

        if len(rvsim_sig) == len(spike_sig) and rvsim_sig == spike_sig:
            return dict(name=name, status="pass", seconds=round(time.time() - t0, 2))

        n = min(len(rvsim_sig), len(spike_sig))
        diff_at = next((i for i in range(n) if rvsim_sig[i] != spike_sig[i]), n)
        return dict(
            name=name,
            status="fail",
            seconds=round(time.time() - t0, 2),
            diff_at=diff_at,
            spike_len=len(spike_sig),
            rvsim_len=len(rvsim_sig),
        )
    except subprocess.TimeoutExpired:
        return dict(name=name, status="timeout", seconds=TIMEOUT_SEC)
    except Exception as e:
        return dict(name=name, status="error", reason=f"{type(e).__name__}: {e}")
    finally:
        for p in (spike_sig_path, rvsim_sig_path):
            try:
                os.remove(p)
            except OSError:
                pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--march",
        default="rv64gcv_zfh_zvfh_zvbb_zvbc_zvkg_zvkned_zvknha_zvksed_zvksh",
    )
    ap.add_argument("--build-dir", default=os.path.join(BUILDS, "vector"))
    ap.add_argument("--filter", default="*")
    ap.add_argument("--smoke", action="store_true", help="run first 20 only")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(BUILDS, "results"))
    args = ap.parse_args()

    if not os.path.isfile(SPIKE):
        sys.exit(
            f"ERROR: local spike not found at {SPIKE}\n"
            f"Run: make vector-test-build (which builds it)"
        )
    if not os.path.isfile(RVSIM_WORKER):
        sys.exit(f"ERROR: rvsim worker not found at {RVSIM_WORKER}")

    pattern = args.filter if args.filter.endswith(".elf") else args.filter + "*.elf"
    elfs = sorted(glob.glob(os.path.join(args.build_dir, f"vlen{args.vlen}", pattern)))
    if args.smoke:
        elfs = elfs[:20]
    if not elfs:
        sys.exit(
            f"ERROR: no ELFs matching '{pattern}' under "
            f"{args.build_dir}/vlen{args.vlen}/\n"
            f"Run build_tests.sh first."
        )

    os.makedirs(args.out, exist_ok=True)
    scratch = os.path.join(args.out, "scratch")
    os.makedirs(scratch, exist_ok=True)
    out_file = os.path.join(args.out, f"results-vlen{args.vlen}.json")

    print(f"Running {len(elfs)} vector tests (vlen={args.vlen}, jobs={args.jobs})")
    print(f"streaming results to: {out_file}")
    results = []
    counts = {}

    def write_partial():
        with open(out_file, "w") as f:
            json.dump(
                {
                    "vlen": args.vlen,
                    "march": args.march,
                    "total_planned": len(elfs),
                    "completed": len(results),
                    "counts": counts,
                    "results": sorted(results, key=lambda r: r["name"]),
                },
                f,
                indent=2,
            )

    try:
        with cf.ProcessPoolExecutor(max_workers=args.jobs) as ex:
            futs = {
                ex.submit(run_one, (e, args.vlen, args.march, scratch)): e
                for e in elfs
            }
            for i, fut in enumerate(cf.as_completed(futs), 1):
                try:
                    r = fut.result()
                except Exception as e:
                    elf = futs[fut]
                    r = dict(
                        name=os.path.basename(elf)[:-4],
                        status="error",
                        reason=f"{type(e).__name__}: {e}",
                    )
                results.append(r)
                counts[r["status"]] = counts.get(r["status"], 0) + 1
                mark = {
                    "pass": ".",
                    "fail": "F",
                    "timeout": "T",
                    "skip": "s",
                    "error": "E",
                }.get(r["status"], "?")
                extra = ""
                if r["status"] == "fail":
                    extra = f" diff@{r.get('diff_at', '?')}"
                elif r["status"] in ("skip", "error"):
                    extra = f" ({(r.get('reason') or '')[:60]})"
                print(f"[{i:5}/{len(elfs)}] {mark} {r['name']}{extra}", flush=True)
                # Stream incremental results every 50 tests so a crash leaves
                # something behind to triage from.
                if i % 50 == 0:
                    write_partial()
    finally:
        write_partial()

    print()
    print(f"=== vlen={args.vlen}: {len(results)} / {len(elfs)} completed ===")
    for k in ("pass", "fail", "timeout", "skip", "error"):
        if k in counts:
            print(f"  {k:8} {counts[k]}")
    print(f"results: {out_file}")

    failed = counts.get("fail", 0) + counts.get("timeout", 0) + counts.get("error", 0)
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
