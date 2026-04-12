#!/usr/bin/env python3
"""Run RVV tests on rvsim and diff against spike (cosim).

For each test ELF under build/vlen{VLEN}/, this script:
  1. Executes it on rvsim and dumps the begin_signature..end_signature region.
  2. Executes it on a local spike and dumps the same region.
  3. Compares the two — pass iff identical.

The local spike binary is testing/vector/third_party/spike-install/bin/spike,
built from source so it supports the modern Z-extensions (zvfh, zvbb, etc.)
that the system spike doesn't.

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

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, REPO_ROOT)

from rvsim import Config, Backend  # noqa: E402
from rvsim._core import Cpu  # noqa: E402
from rvsim.config import _config_to_dict  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SPIKE = os.path.join(HERE, "third_party", "spike-install", "bin", "spike")
READELF = "riscv64-elf-readelf"
CYCLE_LIMIT = 10_000_000


def get_signature_range(elf_path):
    """Return (begin, end) addresses for the signature region."""
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


def run_rvsim(elf_path, vlen, begin, end):
    cfg = Config(width=1, backend=Backend.InOrder(), vlen=vlen)
    with open(elf_path, "rb") as f:
        elf_data = f.read()
    cpu = Cpu(_config_to_dict(cfg), elf_data=elf_data)
    exit_code = cpu.run(limit=CYCLE_LIMIT, stats_sections=None)
    if exit_code is None:
        return None, "timeout"
    sig = bytes(cpu.read_phys_bytes(begin, end - begin))
    return sig, "ok" if exit_code == 0 else f"exit={exit_code}"


def run_spike(elf_path, vlen, march, sig_path):
    os.makedirs(os.path.dirname(sig_path), exist_ok=True)
    # Stripped chipsalliance tests never call RVTEST_PASS, so spike will exit
    # non-zero. We don't care — we only care about the resultdata bytes spike
    # wrote into memory before halting (which spike still dumps via +signature).
    isa = f"{march}_zvl{vlen}b" if f"zvl{vlen}b" not in march else march
    subprocess.run(
        [
            SPIKE,
            f"--isa={isa}",
            f"+signature={sig_path}",
            "+signature-granularity=4",
            elf_path,
        ],
        capture_output=True,
        text=True,
        timeout=300,
    )
    if not os.path.isfile(sig_path):
        return None, "spike produced no signature"
    # spike +signature output is hex words, one per line. Convert to bytes.
    bs = bytearray()
    with open(sig_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            word = int(line, 16)
            bs.extend(struct.pack("<I", word))
    return bytes(bs), "ok"


def run_one(args):
    elf_path, vlen, march, scratch = args
    name = os.path.basename(elf_path)[:-4]
    t0 = time.time()
    try:
        begin, end = get_signature_range(elf_path)
        if begin is None or end is None:
            return dict(name=name, status="skip", reason="no signature symbols")

        sig_spike_path = os.path.join(scratch, f"{name}.spike.sig")
        spike_sig, spike_msg = run_spike(elf_path, vlen, march, sig_spike_path)
        if spike_sig is None:
            return dict(name=name, status="skip", reason=spike_msg)

        rvsim_sig, rvsim_msg = run_rvsim(elf_path, vlen, begin, end)
        if rvsim_msg == "timeout":
            return dict(name=name, status="timeout", seconds=round(time.time() - t0, 2))

        # spike's signature region is what's listed by ELF symbols too — they
        # should be the same length. If not, truncate to the shorter for diff.
        n = min(len(rvsim_sig), len(spike_sig))
        if rvsim_sig[:n] == spike_sig[:n] and len(rvsim_sig) == len(spike_sig):
            return dict(name=name, status="pass", seconds=round(time.time() - t0, 2))
        # Find first differing offset for the report.
        diff_at = next((i for i in range(n) if rvsim_sig[i] != spike_sig[i]), n)
        return dict(
            name=name,
            status="fail",
            seconds=round(time.time() - t0, 2),
            diff_at=diff_at,
            rvsim_msg=rvsim_msg,
        )
    except Exception as e:
        return dict(name=name, status="error", reason=f"{type(e).__name__}: {e}")
    finally:
        # clean up spike sig
        try:
            os.remove(os.path.join(scratch, f"{name}.spike.sig"))
        except OSError:
            pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--march",
        default="rv64gcv_zfh_zvfh_zvbb_zvbc_zvkg_zvkned_zvknha_zvksed_zvksh",
    )
    ap.add_argument("--build-dir", default=os.path.join(HERE, "build"))
    ap.add_argument("--filter", default="*")
    ap.add_argument("--smoke", action="store_true", help="run first 20 only")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(HERE, "results"))
    args = ap.parse_args()

    if not os.path.isfile(SPIKE):
        sys.exit(
            f"ERROR: local spike not found at {SPIKE}\n"
            f"Run: bash testing/vector/build_tests.sh (which builds it)"
        )

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

    print(f"Running {len(elfs)} vector tests (vlen={args.vlen}, jobs={args.jobs})")
    results = []
    with cf.ProcessPoolExecutor(max_workers=args.jobs) as ex:
        futs = {
            ex.submit(run_one, (e, args.vlen, args.march, scratch)): e for e in elfs
        }
        for i, fut in enumerate(cf.as_completed(futs), 1):
            r = fut.result()
            results.append(r)
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
                extra = f" ({r.get('reason', '')[:60]})"
            print(f"[{i:5}/{len(elfs)}] {mark} {r['name']}{extra}")

    counts = {}
    for r in results:
        counts[r["status"]] = counts.get(r["status"], 0) + 1

    summary = {
        "vlen": args.vlen,
        "march": args.march,
        "total": len(results),
        "counts": counts,
        "results": sorted(results, key=lambda r: r["name"]),
    }
    out_file = os.path.join(args.out, f"results-vlen{args.vlen}.json")
    with open(out_file, "w") as f:
        json.dump(summary, f, indent=2)

    print()
    print(f"=== vlen={args.vlen}: {len(results)} total ===")
    for k in ("pass", "fail", "timeout", "skip", "error"):
        if k in counts:
            print(f"  {k:8} {counts[k]}")
    print(f"results: {out_file}")

    failed = counts.get("fail", 0) + counts.get("timeout", 0) + counts.get("error", 0)
    sys.exit(0 if failed == 0 else 1)


if __name__ == "__main__":
    main()
