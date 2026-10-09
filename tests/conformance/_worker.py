#!/usr/bin/env python3
"""Single-test rvsim worker, used by every multi-config test runner.

Runs one ELF on rvsim with a named PIPELINES config and (optionally) dumps the
ELF's begin_signature..end_signature region as spike's +signature does.

Used as a subprocess by the multi-config runners so a single panic / segfault
in the simulator only kills one test, never the whole sweep.

Usage:
    _worker.py <elf> <pipeline_label> [<sig_out_path>] [--commit-log PATH]

With --commit-log, every retired instruction is written to PATH (see
Simulator.open_commit_log) for tests/conformance/lockstep.py to replay on spike.

Exit codes:
    0   pass (cpu.run() returned 0)
    1   fail (cpu.run() returned non-zero exit code)
    124 timeout (cycle budget exhausted before HTIF exit)
    2   bad usage / unknown pipeline label
"""

import argparse
import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from rvsim._core import Simulator
from rvsim.config._config import _config_to_dict
from tests.conformance.configs.pipelines import PIPELINES

CYCLE_LIMIT = int(os.environ.get("RVSIM_CYCLE_LIMIT", "10000000"))
READELF = "riscv64-elf-readelf"


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


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("elf")
    parser.add_argument("pipeline_label")
    parser.add_argument("sig_out", nargs="?")
    parser.add_argument("--commit-log")
    args = parser.parse_args()

    elf_path = args.elf
    label = args.pipeline_label
    sig_out = args.sig_out

    cfg = next((c for lbl, c in PIPELINES if lbl == label), None)
    if cfg is None:
        print(f"unknown pipeline label: {label}", file=sys.stderr)
        sys.exit(2)

    # Multi-hart runs: secondary harts park in the riscv-tests environment
    # (`csrr a0, mhartid; bnez a0, .`), so every ISA test doubles as a check
    # that extra harts neither crash nor disturb hart 0.
    hart_count = os.environ.get("RVSIM_HART_COUNT")
    if hart_count:
        cfg.hart_count = int(hart_count)

    # Vector cosim tests are built for a specific VLEN (the path is
    # ``vlen{N}/test.elf``). The chipsalliance test ELF embeds VLEN-dependent
    # data layout, so a config with a different VLEN (e.g. ``ref linux`` at
    # VLEN=256) silently mismatches every vse store. Pin VLEN to the path
    # when we can detect it.
    import re

    m = re.search(r"/vlen(\d+)/", elf_path)
    if m:
        cfg.vlen = int(m.group(1))

    with open(elf_path, "rb") as f:
        elf_data = f.read()
    cpu = Simulator(_config_to_dict(cfg), elf_data=elf_data)
    if args.commit_log:
        cpu.open_commit_log(args.commit_log)
    exit_code = cpu.run(limit=CYCLE_LIMIT, stats_sections=None)

    if sig_out:
        begin, end = get_signature_range(elf_path)
        if begin is not None and end is not None and end > begin:
            sig_bytes = bytes(cpu.read_phys_bytes(begin, end - begin))
            with open(sig_out, "w") as f:
                for i in range(0, len(sig_bytes), 4):
                    word = struct.unpack_from("<I", sig_bytes, i)[0]
                    f.write(f"{word:08x}\n")

    # Dropping the simulator flushes the commit log.
    del cpu

    if exit_code is None:
        sys.exit(124)
    sys.exit(0 if exit_code == 0 else 1)


if __name__ == "__main__":
    main()
