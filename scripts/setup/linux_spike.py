#!/usr/bin/env python3
"""Compare rvsim Linux boot against spike commit trace.

Runs spike and rvsim side by side (both booting OpenSBI + Linux), captures
commit traces, and reports the first divergence point.  This is the primary
debugging tool for ISA compliance bugs that cause the Linux boot crash.

Spike uses its built-in OpenSBI and HTIF console.  rvsim uses the external
fw_jump.bin with UART/PLIC.  Both load the same kernel Image.  The commit
traces will eventually diverge when the kernel starts probing different
devices, but any *early* divergence points to a real ISA bug.

Usage (from repo root, must have built with `maturin develop --release --features commit-log`):
    .venv/bin/python scripts/setup/linux_spike.py                  # full run
    .venv/bin/python scripts/setup/linux_spike.py --skip-spike     # reuse cached spike log
    .venv/bin/python scripts/setup/linux_spike.py --spike-only     # only generate spike trace
    .venv/bin/python scripts/setup/linux_spike.py --limit 5000000  # limit rvsim cycles
    .venv/bin/python scripts/setup/linux_spike.py --context 20     # show 20 lines of context
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LINUX_DIR = os.path.join(ROOT, "software", "linux")
OUTPUT_DIR = os.path.join(LINUX_DIR, "output")
SPIKE_LOG = os.path.join(LINUX_DIR, ".spike-linux-trace.log")

FW_JUMP_ELF = os.path.join(
    LINUX_DIR, "buildroot-2024.08", "output", "images", "fw_jump.elf"
)
KERNEL_IMAGE = os.path.join(OUTPUT_DIR, "Image")

# ── Log parsing ─────────────────────────────────────────────────────────────
LINE_RE = re.compile(r"core\s+\d+:\s+0x([0-9a-f]+)\s+\(0x([0-9a-f]+)\)")
NOP_ENCODINGS = {0x00000013, 0x00000001}

# Spike reset vector range (bootrom at 0x1000-0x1fff).
SPIKE_RESET_START = 0x1000
SPIKE_RESET_END = 0x2000


def parse_log(path, skip_nops=True, skip_reset=False, max_entries=0):
    """Parse a commit log into a list of (pc, inst) tuples."""
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
    """Compare two traces by PC. Returns (match_count, divergence_info | None)."""
    for i, (s, r) in enumerate(zip(spike_trace, rvsim_trace)):
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


# ── Spike oracle ────────────────────────────────────────────────────────────


def generate_spike_trace(spike_limit):
    """Run spike -l booting Linux, capture commit trace to SPIKE_LOG.

    spike boots OpenSBI (from fw_jump.elf) which jumps to the kernel Image.
    The -l flag logs every retired instruction to stderr.

    We run spike for `spike_limit` instructions then kill it (Linux boot
    on spike takes tens of billions of instructions; we only need enough
    to find the first divergence).
    """
    if not os.path.isfile(FW_JUMP_ELF):
        print(f"ERROR: {FW_JUMP_ELF} not found. Run boot_linux.py first.")
        return False
    if not os.path.isfile(KERNEL_IMAGE):
        print(f"ERROR: {KERNEL_IMAGE} not found. Run boot_linux.py first.")
        return False

    print(f"[spike] Booting Linux (capturing up to {spike_limit:,} instructions)...")
    print(f"[spike] fw_jump.elf: {FW_JUMP_ELF}")
    print(f"[spike] kernel:      {KERNEL_IMAGE}")

    cmd = [
        "spike",
        "-l",
        "--isa=rv64gc",
        f"-m0x80000000:0x10000000",  # 256MB RAM at 0x80000000
        f"--kernel={KERNEL_IMAGE}",
        FW_JUMP_ELF,
    ]

    start = time.time()
    try:
        # Run spike, capturing stderr (commit log) to file.
        # spike runs forever booting Linux, so we kill it after enough output.
        with open(SPIKE_LOG, "wb") as logfile:
            proc = subprocess.Popen(
                cmd,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            )

            count = 0
            buf = bytearray()
            try:
                while True:
                    chunk = proc.stderr.read(65536)
                    if not chunk:
                        break
                    buf.extend(chunk)
                    # Count instruction lines in this chunk
                    count += chunk.count(b"\n")
                    logfile.write(chunk)
                    if count >= spike_limit:
                        break
            finally:
                proc.kill()
                proc.wait()

    except FileNotFoundError:
        print("ERROR: 'spike' not found in PATH")
        return False

    elapsed = time.time() - start
    # Count actual entries
    entries = parse_log(SPIKE_LOG, skip_nops=True, skip_reset=True)
    print(
        f"[spike] Captured {len(entries):,} instructions in {elapsed:.1f}s -> {SPIKE_LOG}"
    )
    return True


# ── rvsim run ───────────────────────────────────────────────────────────────


def run_rvsim_trace(cycle_limit):
    """Boot Linux in rvsim with commit logging, return parsed trace."""
    # Import here so the script can show --help without building
    from rvsim._core import Cpu
    from rvsim.config import _config_to_dict

    # Use the same linux boot config
    sys.path.insert(0, os.path.join(ROOT, "scripts", "setup"))
    from boot_linux import config as linux_config

    cfg = linux_config()
    config_dict = _config_to_dict(cfg)

    fd, log_path = tempfile.mkstemp(suffix=".log", prefix="rvsim_linux_")
    os.close(fd)

    print(f"[rvsim] Booting Linux ({cycle_limit:,} cycle limit)...")

    try:
        cpu = Cpu(
            config_dict,
            kernel_path=KERNEL_IMAGE,
            dtb_path=os.path.join(LINUX_DIR, "system.dtb"),
            disk_path=os.path.join(OUTPUT_DIR, "disk.img"),
        )
        cpu.open_commit_log(log_path)

        start = time.time()
        cpu.run(limit=cycle_limit, stats_sections=None)
        elapsed = time.time() - start
        del cpu  # flush BufWriter

        trace = parse_log(log_path, skip_nops=True)
        print(f"[rvsim] Retired {len(trace):,} instructions in {elapsed:.1f}s")
    finally:
        if os.path.exists(log_path):
            os.unlink(log_path)

    return trace


# ── Disassembly helper ──────────────────────────────────────────────────────


def try_disasm(pc, inst):
    """Try to disassemble a single instruction using objdump."""
    # 32-bit instruction
    inst_bytes = inst.to_bytes(4, "little")
    try:
        with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as f:
            f.write(inst_bytes)
            tmp = f.name
        result = subprocess.run(
            [
                "riscv64-linux-gnu-objdump",
                "-D",
                "-b",
                "binary",
                "-m",
                "riscv:rv64",
                tmp,
            ],
            capture_output=True,
            text=True,
            timeout=5,
        )
        os.unlink(tmp)
        for line in result.stdout.splitlines():
            if ":" in line and "\t" in line:
                # Extract just the mnemonic
                parts = line.split("\t")
                if len(parts) >= 3:
                    return parts[2].strip()
        return ""
    except Exception:
        return ""


# ── Main ────────────────────────────────────────────────────────────────────


def main():
    ap = argparse.ArgumentParser(
        description="Compare rvsim Linux boot against spike commit trace"
    )
    ap.add_argument(
        "--spike-only", action="store_true", help="Only generate spike trace"
    )
    ap.add_argument(
        "--skip-spike", action="store_true", help="Reuse cached spike trace"
    )
    ap.add_argument(
        "--spike-limit",
        type=int,
        default=50_000_000,
        help="Max instruction lines to capture from spike (default: 50M)",
    )
    ap.add_argument(
        "--limit",
        type=int,
        default=500_000_000,
        help="rvsim cycle limit (default: 500M)",
    )
    ap.add_argument(
        "--context",
        type=int,
        default=10,
        help="Number of instructions to show around divergence (default: 10)",
    )
    ap.add_argument(
        "--no-disasm",
        action="store_true",
        help="Skip disassembly (faster)",
    )
    args = ap.parse_args()

    # ── Phase 1: Spike trace ────────────────────────────────────────────
    if not args.skip_spike:
        ok = generate_spike_trace(args.spike_limit)
        if not ok:
            return 1
    else:
        if not os.path.isfile(SPIKE_LOG):
            print(f"ERROR: No cached spike trace at {SPIKE_LOG}")
            print("       Run without --skip-spike first.")
            return 1
        print(f"[spike] Using cached trace: {SPIKE_LOG}")

    if args.spike_only:
        return 0

    # ── Phase 2: rvsim trace ────────────────────────────────────────────
    rvsim_trace = run_rvsim_trace(args.limit)
    if not rvsim_trace:
        print("ERROR: rvsim produced no commit log entries")
        return 1

    # ── Phase 3: Load spike trace and compare ───────────────────────────
    # Only load as many spike entries as we need
    spike_trace = parse_log(SPIKE_LOG, skip_nops=True, skip_reset=True, max_entries=len(rvsim_trace) + 1000)
    print(f"\n[compare] spike: {len(spike_trace):,} instructions")
    print(f"[compare] rvsim: {len(rvsim_trace):,} instructions")
    print()

    matched, div = compare_traces(spike_trace, rvsim_trace)

    if div is None:
        print("=" * 72)
        print(f"MATCH — {matched:,} instructions agree (rvsim trace is shorter or equal)")
        print("=" * 72)
        if len(rvsim_trace) < len(spike_trace):
            print(
                f"\nrvsim stopped after {len(rvsim_trace):,} instructions "
                f"(spike has {len(spike_trace):,}). "
                f"Increase --limit to compare further."
            )
        return 0

    # ── Divergence found ────────────────────────────────────────────────
    if div.get("length_mismatch"):
        print("=" * 72)
        print(
            f"LENGTH MISMATCH at inst #{div['inst_num']:,} — "
            f"rvsim retired more instructions ({div['rvsim_len']:,}) than spike ({div['spike_len']:,})"
        )
        print("=" * 72)
        return 1

    inst_num = div["inst_num"]
    print("=" * 72)
    print(f"DIVERGENCE at instruction #{inst_num:,}")
    print(f"  spike: PC=0x{div['spike_pc']:016x}  inst=0x{div['spike_inst']:08x}")
    print(f"  rvsim: PC=0x{div['rvsim_pc']:016x}  inst=0x{div['rvsim_inst']:08x}")
    print("=" * 72)

    # Show context around divergence
    ctx = args.context
    start = max(0, inst_num - 1 - ctx)
    end = min(len(spike_trace), len(rvsim_trace), inst_num - 1 + ctx)

    print(f"\n{'idx':>8s}  {'spike PC':>18s} {'spike inst':>12s}  {'rvsim PC':>18s} {'rvsim inst':>12s}  status")
    print("-" * 95)
    for i in range(start, end):
        s_pc, s_inst = spike_trace[i]
        r_pc, r_inst = rvsim_trace[i]
        if s_pc == r_pc:
            status = "  ok"
        else:
            status = "  <<< DIVERGE"
        disasm = ""
        if not args.no_disasm and s_pc == r_pc:
            disasm = try_disasm(s_pc, s_inst)
            if disasm:
                disasm = f"  {disasm}"

        print(
            f"{i+1:>8,d}  0x{s_pc:016x} (0x{s_inst:08x})  0x{r_pc:016x} (0x{r_inst:08x}){status}{disasm}"
        )

    # Try to identify what kind of divergence this is
    print("\n── Analysis ──")
    s_pc = div["spike_pc"]
    r_pc = div["rvsim_pc"]

    # Check if spike jumped to a trap vector
    if s_pc in (0x80000000, 0x80000004):
        print(f"  spike jumped to trap handler at 0x{s_pc:x}")
        print(f"  rvsim continued to 0x{r_pc:x} (missed the trap?)")
    elif r_pc in (0x80000000, 0x80000004):
        print(f"  rvsim jumped to trap handler at 0x{r_pc:x}")
        print(f"  spike continued to 0x{s_pc:x} (spurious trap in rvsim?)")
    else:
        # Check if one of them took a different branch
        prev_pc = spike_trace[inst_num - 2][0] if inst_num >= 2 else 0
        s_delta = s_pc - prev_pc
        r_delta = r_pc - prev_pc
        if s_delta != r_delta:
            print(
                f"  Previous PC: 0x{prev_pc:016x}"
            )
            print(
                f"  spike went to 0x{s_pc:x} (delta={s_delta:+d}), "
                f"rvsim went to 0x{r_pc:x} (delta={r_delta:+d})"
            )
            if abs(s_delta) > 0x100 or abs(r_delta) > 0x100:
                print("  Looks like a branch/jump divergence (different target or taken/not-taken)")
            else:
                print("  Looks like a different instruction fetch (possibly different memory contents)")

    # Show a few more instructions from each trace after divergence to help debug
    print(f"\n── Spike trace after divergence (next {ctx} instructions) ──")
    for i in range(inst_num - 1, min(len(spike_trace), inst_num - 1 + ctx)):
        pc, inst = spike_trace[i]
        disasm = ""
        if not args.no_disasm:
            disasm = try_disasm(pc, inst)
            if disasm:
                disasm = f"  {disasm}"
        print(f"  {i+1:>8,d}  0x{pc:016x} (0x{inst:08x}){disasm}")

    print(f"\n── rvsim trace after divergence (next {ctx} instructions) ──")
    for i in range(inst_num - 1, min(len(rvsim_trace), inst_num - 1 + ctx)):
        pc, inst = rvsim_trace[i]
        disasm = ""
        if not args.no_disasm:
            disasm = try_disasm(pc, inst)
            if disasm:
                disasm = f"  {disasm}"
        print(f"  {i+1:>8,d}  0x{pc:016x} (0x{inst:08x}){disasm}")

    return 1


if __name__ == "__main__":
    sys.exit(main())
