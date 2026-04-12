#!/usr/bin/env python3
"""Single-test rvsim worker, used by every multi-config test runner.

Runs one ELF on rvsim with a named PIPELINES config and (optionally) dumps the
ELF's begin_signature..end_signature region in the riscof hex format.

Used as a subprocess by the multi-config runners so a single panic / segfault
in the simulator only kills one test, never the whole sweep.

Usage:
    _worker.py <elf> <pipeline_label> [<sig_out_path>]

Exit codes:
    0   pass (cpu.run() returned 0)
    1   fail (cpu.run() returned non-zero exit code)
    124 timeout (cycle budget exhausted before HTIF exit)
    2   bad usage / unknown pipeline label
"""

import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from rvsim._core import Cpu  # noqa: E402
from rvsim.config import _config_to_dict  # noqa: E402
from testing.configs.pipelines import PIPELINES  # noqa: E402

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
    if len(sys.argv) < 3:
        print("usage: _worker.py <elf> <pipeline_label> [<sig_out>]", file=sys.stderr)
        sys.exit(2)

    elf_path = sys.argv[1]
    label = sys.argv[2]
    sig_out = sys.argv[3] if len(sys.argv) > 3 else None

    cfg = next((c for lbl, c in PIPELINES if lbl == label), None)
    if cfg is None:
        print(f"unknown pipeline label: {label}", file=sys.stderr)
        sys.exit(2)

    with open(elf_path, "rb") as f:
        elf_data = f.read()
    cpu = Cpu(_config_to_dict(cfg), elf_data=elf_data)
    exit_code = cpu.run(limit=CYCLE_LIMIT, stats_sections=None)

    if sig_out:
        begin, end = get_signature_range(elf_path)
        if begin is not None and end is not None and end > begin:
            sig_bytes = bytes(cpu.read_phys_bytes(begin, end - begin))
            with open(sig_out, "w") as f:
                for i in range(0, len(sig_bytes), 4):
                    word = struct.unpack_from("<I", sig_bytes, i)[0]
                    f.write(f"{word:08x}\n")

    if exit_code is None:
        sys.exit(124)
    sys.exit(0 if exit_code == 0 else 1)


if __name__ == "__main__":
    main()
