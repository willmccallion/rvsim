"""What the conformance runners share: where the test builds and spike
live, the signature region a test writes, and commit-trace parsing."""

import os
import re
import struct
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BUILDS = os.path.join(ROOT, "tests", "builds")
SPIKE = os.path.join(BUILDS, "spike-install", "bin", "spike")
READELF = "riscv64-elf-readelf"

# One retired instruction in a spike -l / rvsim commit log: its PC and bits.
LINE_RE = re.compile(r"core\s+\d+:\s+0x([0-9a-f]+)\s+\(0x([0-9a-f]+)\)")
# addi x0, x0, 0 and c.nop.
NOP_ENCODINGS = {0x00000013, 0x00000001}
# spike's boot ROM, which rvsim does not run.
SPIKE_RESET_START = 0x1000
SPIKE_RESET_END = 0x2000


def signature_range(elf_path):
    """The (begin, end) addresses of the ELF's begin_signature and
    end_signature symbols, either None when the ELF lacks it."""
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


def read_signature(path):
    """A +signature file, one hex word per line, as bytes."""
    out = bytearray()
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                out.extend(struct.pack("<I", int(line, 16)))
    return bytes(out)


def parse_log(path, skip_nops=True, skip_reset=False, max_entries=0):
    """A commit log as (pc, inst) tuples, at most ``max_entries`` of them
    when it is non-zero."""
    entries = []
    with open(path) as f:
        for line in f:
            m = LINE_RE.search(line)
            if m is None:
                continue
            pc = int(m.group(1), 16)
            inst = int(m.group(2), 16)
            if skip_reset and SPIKE_RESET_START <= pc < SPIKE_RESET_END:
                continue
            if skip_nops and inst in NOP_ENCODINGS:
                continue
            entries.append((pc, inst))
            if max_entries and len(entries) >= max_entries:
                break
    return entries


def compare_traces(spike_trace, rvsim_trace):
    """Compares two traces by PC. Returns (match_count, divergence_info_or_None).

    Only PCs are compared, not instruction encodings, because rvsim logs
    the 32-bit expanded form of compressed instructions while spike logs
    the original 16-bit encoding.

    rvsim may be shorter than spike because rvsim exits immediately on
    HTIF tohost write, while spike continues the polling loop.  A shorter
    rvsim trace is fine as long as every PC it *did* retire matches spike
    exactly.  A shorter *spike* trace (rvsim retired instructions spike
    didn't) is a real divergence.
    """
    for i, (s, r) in enumerate(zip(spike_trace, rvsim_trace, strict=False)):
        if s[0] != r[0]:
            return i, {
                "inst_num": i + 1,
                "spike_pc": s[0],
                "spike_inst": s[1],
                "rvsim_pc": r[0],
                "rvsim_inst": r[1],
            }
    min_len = min(len(spike_trace), len(rvsim_trace))
    if len(rvsim_trace) > len(spike_trace):
        return min_len, {
            "inst_num": min_len + 1,
            "length_mismatch": True,
            "spike_len": len(spike_trace),
            "rvsim_len": len(rvsim_trace),
        }
    return min_len, None
