#!/usr/bin/env python3
"""Triage a single failing vector test by diffing rvsim and spike side by side.

Usage:
    python triage.py <elf_path> [--vlen 128]

Runs the ELF on both rvsim and the local spike, dumps the begin_signature..
end_signature region from each, and shows the first differing offset along
with a hexdump-style window of context. Then prints a tail of spike's commit
log so you can see which vector instructions ran near the divergence.
"""

import argparse
import os
import struct
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
sys.path.insert(0, REPO_ROOT)

from rvsim._core import Simulator
from rvsim.config._config import _config_to_dict
from tests.conformance.common import SPIKE, signature_range
from tests.conformance.configs.pipelines import PIPELINES

HERE = os.path.dirname(os.path.abspath(__file__))


def hex_window(b, offset, before=16, after=32):
    start = max(0, offset - before)
    stop = min(len(b), offset + after)
    bs = b[start:stop]
    out = []
    for i in range(0, len(bs), 16):
        addr = start + i
        chunk = bs[i : i + 16]
        hexs = " ".join(f"{x:02x}" for x in chunk)
        ascs = "".join(chr(x) if 32 <= x < 127 else "." for x in chunk)
        marker = " <" if start + i <= offset < start + i + 16 else "  "
        out.append(f"{addr:08x}{marker} {hexs:<48}  {ascs}")
    return "\n".join(out)


def run_spike(elf: str, isa: str) -> tuple[bytes, str]:
    """Runs ``elf`` on spike with ``isa``; returns its signature and log.

    The files spike writes go in a directory of their own, so concurrent
    runs do not overwrite each other's."""
    with tempfile.TemporaryDirectory(prefix="triage-") as tmp:
        sig_path = os.path.join(tmp, "spike.sig")
        log_path = os.path.join(tmp, "spike.log")
        res = subprocess.run(
            [
                SPIKE,
                f"--isa={isa}",
                f"--log={log_path}",
                "--log-commits",
                f"+signature={sig_path}",
                "+signature-granularity=4",
                elf,
            ],
            capture_output=True,
            text=True,
            timeout=300,
            check=False,
        )
        if not os.path.isfile(sig_path):
            sys.exit(
                f"spike did not produce a signature (rc={res.returncode})\n"
                f"stderr: {res.stderr.strip()[:400]}\n"
                f"isa was: {isa}"
            )
        signature = bytearray()
        with open(sig_path) as f:
            for line in f:
                line = line.strip()
                if line:
                    signature.extend(struct.pack("<I", int(line, 16)))
        with open(log_path) as f:
            log = f.read()
    return bytes(signature), log


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("elf")
    ap.add_argument("--vlen", type=int, default=128)
    ap.add_argument(
        "--march",
        default="rv64gcv_zfh_zvfh_zvbb_zvbc_zvkg_zvkned_zvknha_zvksed_zvksh",
    )
    args = ap.parse_args()

    if not os.path.isfile(args.elf):
        sys.exit(f"no such ELF: {args.elf}")

    begin, end = signature_range(args.elf)
    if begin is None:
        sys.exit("ELF has no begin_signature/end_signature symbols")
    print(f"signature region: {begin:#x}..{end:#x}  ({end - begin} bytes)")

    isa = f"{args.march}_zvl{args.vlen}b"

    spike_sig, spike_log = run_spike(args.elf, isa)

    # ── rvsim ────────────────────────────────────────────────────────────────
    cfg = dict(PIPELINES)["inorder w1"].replace(vlen=args.vlen)
    with open(args.elf, "rb") as f:
        elf_data = f.read()
    cpu = Simulator(_config_to_dict(cfg), elf_data=elf_data)
    rvsim_exit = cpu.run(limit=10_000_000)
    rvsim_sig = bytes(cpu.read_phys_bytes(begin, end - begin))

    print(f"rvsim exit_code: {rvsim_exit}")
    print(f"spike sig: {len(spike_sig)} bytes, rvsim sig: {len(rvsim_sig)} bytes")

    if spike_sig == rvsim_sig:
        print("\nSIGNATURES MATCH — test passes")
        return

    n = min(len(spike_sig), len(rvsim_sig))
    diff = next((i for i in range(n) if spike_sig[i] != rvsim_sig[i]), n)
    print(f"\nFIRST DIFFERENCE at offset {diff} (addr {begin + diff:#x})")

    print("\n--- spike ---")
    print(hex_window(spike_sig, diff))
    print("\n--- rvsim ---")
    print(hex_window(rvsim_sig, diff))

    print("\n--- spike commit log (last 40 vector lines) ---")
    lines = spike_log.splitlines(keepends=True)
    vec_lines = [
        ln
        for ln in lines
        if any(
            v in ln
            for v in (" v", "vsetvl", "vadd", "vsub", "vmul", "vdiv", "vse", "vle")
        )
    ]
    for ln in vec_lines[-40:]:
        print(ln.rstrip())

    sys.exit(1)


if __name__ == "__main__":
    main()
