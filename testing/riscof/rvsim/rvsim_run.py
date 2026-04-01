#!/usr/bin/env python3
"""Run a single arch-test ELF on rvsim and dump the signature region.

Usage:
    python rvsim_run.py <elf> <signature_file>

Loads the ELF, runs it on rvsim, extracts the memory region between
begin_signature and end_signature symbols, and writes it in the hex
format riscof expects (one 32-bit word per line, big-endian hex).
"""

import struct
import subprocess
import sys
import os

# Ensure rvsim is importable from the repo root
REPO_ROOT = os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)
sys.path.insert(0, REPO_ROOT)

from rvsim import Config, Backend
from rvsim._core import Cpu
from rvsim.config import _config_to_dict

CYCLE_LIMIT = 2_000_000
READELF = None


def _find_readelf():
    """Find a riscv readelf binary."""
    import shutil
    for prefix in ("riscv64-elf-", "riscv64-none-elf-", "riscv64-unknown-elf-"):
        path = shutil.which(prefix + "readelf")
        if path:
            return path
    return "readelf"


def get_symbol_addr(elf_path, symbol_name):
    """Get the address of a symbol from the ELF symbol table."""
    global READELF
    if READELF is None:
        READELF = _find_readelf()

    result = subprocess.run(
        [READELF, "-s", elf_path], capture_output=True, text=True
    )
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[-1] == symbol_name:
            try:
                return int(parts[1], 16)
            except ValueError:
                continue
    return None


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <elf> <signature_file>", file=sys.stderr)
        sys.exit(1)

    elf_path = sys.argv[1]
    sig_file = sys.argv[2]

    # Get signature region addresses from ELF
    begin_sig = get_symbol_addr(elf_path, "begin_signature")
    end_sig = get_symbol_addr(elf_path, "end_signature")

    if begin_sig is None or end_sig is None:
        print(
            f"ERROR: Could not find begin_signature/end_signature in {elf_path}",
            file=sys.stderr,
        )
        sys.exit(1)

    sig_size = end_sig - begin_sig
    if sig_size <= 0:
        print(
            f"ERROR: Invalid signature region: {begin_sig:#x} - {end_sig:#x}",
            file=sys.stderr,
        )
        sys.exit(1)

    # Build a simple config for fast execution
    cfg = Config(width=4, backend=Backend.InOrder())
    config_dict = _config_to_dict(cfg)

    # Load and run
    with open(elf_path, "rb") as f:
        elf_data = f.read()

    cpu = Cpu(config_dict, elf_data=elf_data)
    exit_code = cpu.run(limit=CYCLE_LIMIT, stats_sections=None)

    if exit_code is None:
        print(
            f"WARNING: Simulation did not exit within {CYCLE_LIMIT} cycles: {elf_path}",
            file=sys.stderr,
        )

    # Extract signature from memory
    sig_bytes = bytes(cpu.read_phys_bytes(begin_sig, sig_size))

    # Write signature in riscof format: one 32-bit word per line, big-endian hex
    with open(sig_file, "w") as f:
        for offset in range(0, len(sig_bytes), 4):
            word = struct.unpack_from("<I", sig_bytes, offset)[0]
            f.write(f"{word:08x}\n")


if __name__ == "__main__":
    main()
