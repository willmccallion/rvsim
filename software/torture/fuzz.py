#!/usr/bin/env python3
"""Deep fuzzing: signature comparison between Spike and rvsim.

Unlike run.py which only compares exit codes, this script:
  1. Generates tests with a signature region
  2. Runs on Spike with +signature to dump final register/memory state
  3. Runs on rvsim and reads signature from memory via the Python API
  4. Compares byte-for-byte, reporting exact register mismatches

This catches silent data corruption that doesn't affect the exit code.

Usage (from repo root):
    .venv/bin/python3 software/torture/fuzz.py --seed 0 --count 500
    .venv/bin/python3 software/torture/fuzz.py --seed 0 --count 100 --config linux
    .venv/bin/python3 software/torture/fuzz.py --skip-generate --config small
"""

import argparse
import contextlib
import io
import os
import shutil
import struct
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TORTURE_DIR = os.path.dirname(os.path.abspath(__file__))
GEN_DIR = os.path.join(TORTURE_DIR, "generated")
BUILD_DIR = os.path.join(TORTURE_DIR, "build")
FAIL_DIR = os.path.join(TORTURE_DIR, "failures")

RISCV_TESTS_DIR = os.path.join(REPO_ROOT, "software", "riscv-tests")
ENV_DIR = os.path.join(RISCV_TESTS_DIR, "env")
ISA_MACROS = os.path.join(RISCV_TESTS_DIR, "isa", "macros", "scalar")
LINK_SCRIPT = os.path.join(ENV_DIR, "p", "link.ld")

CC = "riscv64-elf-gcc"
READELF = "riscv64-elf-readelf"
SPIKE = "spike"

# Registers dumped in signature (x5-x31, skipping x4/tp)
SIG_REGS = [r for r in range(5, 32)]
SIG_SIZE = len(SIG_REGS) * 8  # bytes

ABI_NAMES = {
    0: "zero", 1: "ra", 2: "sp", 3: "gp", 4: "tp",
    5: "t0", 6: "t1", 7: "t2", 8: "s0", 9: "s1",
    10: "a0", 11: "a1", 12: "a2", 13: "a3", 14: "a4",
    15: "a5", 16: "a6", 17: "a7",
    18: "s2", 19: "s3", 20: "s4", 21: "s5", 22: "s6",
    23: "s7", 24: "s8", 25: "s9", 26: "s10", 27: "s11",
    28: "t3", 29: "t4", 30: "t5", 31: "t6",
}


def assemble_test(src_path, elf_path):
    """Assemble a torture .S file into an ELF."""
    cmd = [
        CC,
        "-march=rv64gc", "-mabi=lp64d",
        "-static", "-mcmodel=medany",
        "-fvisibility=hidden", "-nostdlib", "-nostartfiles",
        f"-I{os.path.join(ENV_DIR, 'p')}",
        f"-I{ENV_DIR}",
        f"-I{ISA_MACROS}",
        f"-T{LINK_SCRIPT}",
        "-o", elf_path,
        src_path,
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        return False
    return True


def get_symbol_addr(elf_path, symbol_name):
    """Get the address of a symbol from the ELF symbol table."""
    result = subprocess.run(
        [READELF, "-s", elf_path], capture_output=True, text=True
    )
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[-1] == symbol_name:
            try:
                addr = int(parts[1], 16)
                if addr > 0:
                    return addr
            except ValueError:
                continue
    return None


def run_spike_signature(elf_path):
    """Run test on Spike and extract the signature region.

    Returns (exit_code, dict of {register_index: value}).
    """
    with tempfile.NamedTemporaryFile(suffix=".sig", delete=False) as sigf:
        sig_path = sigf.name

    try:
        result = subprocess.run(
            [SPIKE, "--isa=rv64gc", f"+signature={sig_path}", elf_path],
            capture_output=True, text=True, timeout=30,
        )
    except subprocess.TimeoutExpired:
        return -1, None

    sig_values = None
    if os.path.exists(sig_path):
        try:
            with open(sig_path, "r") as f:
                raw_hex = f.read().strip().replace("\n", "")
            # Spike dumps the signature region as big-endian hex.
            # Convert to bytes, then extract 64-bit LE values (as stored in memory).
            sig_bytes = bytes.fromhex(raw_hex)
            sig_values = {}
            for i, reg in enumerate(SIG_REGS):
                offset = i * 8
                if offset + 8 <= len(sig_bytes):
                    # Spike outputs big-endian hex of memory, but RISC-V is LE.
                    # The hex string represents bytes in address order (low to high),
                    # but each 128-bit line is printed MSB first. We need to reverse
                    # within each 16-byte chunk.
                    # Actually: Spike's +signature prints each 16-byte chunk as a
                    # big-endian 128-bit integer. So bytes at addr+0..+15 become
                    # hex digits [15][14]...[1][0].
                    chunk_idx = offset // 16
                    chunk_offset = offset % 16
                    chunk_start = chunk_idx * 32  # hex chars
                    # The chunk is big-endian: first hex char = highest byte
                    # byte at addr+0 is at hex position 30..31
                    # byte at addr+k is at hex position 2*(15-k)..2*(15-k)+1
                    val = 0
                    for b in range(8):
                        byte_in_chunk = chunk_offset + b
                        hex_pos = chunk_start + 2 * (15 - byte_in_chunk)
                        if hex_pos + 2 <= len(raw_hex):
                            byte_val = int(raw_hex[hex_pos:hex_pos + 2], 16)
                            val |= byte_val << (b * 8)
                    sig_values[reg] = val
        except Exception:
            pass
        os.unlink(sig_path)
    else:
        # Try without signature file - just use exit code
        pass

    return result.returncode, sig_values


def run_rvsim_signature(elf_path, config_name="default"):
    """Run test on rvsim and extract signature from memory.

    Returns (exit_code, dict of {register_index: value}).
    """
    sys.path.insert(0, REPO_ROOT)
    from rvsim import Backend, Config, Simulator

    cfg = make_config(config_name)
    sim = Simulator().config(cfg).binary(elf_path)
    cpu = sim.build()

    with (
        contextlib.redirect_stdout(io.StringIO()),
        contextlib.redirect_stderr(io.StringIO()),
    ):
        rc = cpu.run(limit=1_000_000)

    # Read signature from memory
    sig_addr = get_symbol_addr(elf_path, "begin_signature")
    sig_values = None
    if sig_addr is not None and rc == 0:
        sig_values = {}
        for i, reg in enumerate(SIG_REGS):
            addr = sig_addr + i * 8
            try:
                val = cpu.mem64[addr]
                sig_values[reg] = val
            except Exception:
                break

    return rc, sig_values


def make_config(config_name):
    """Create an rvsim Config for the given config name."""
    from rvsim import (
        Backend, BranchPredictor, Cache, Config, Fu,
        MemoryController, Prefetcher, ReplacementPolicy,
    )

    if config_name == "default":
        return Config(width=4, backend=Backend.OutOfOrder())
    elif config_name == "wide":
        return Config(width=8, backend=Backend.OutOfOrder(
            rob_size=256, issue_queue_size=96,
            store_buffer_size=64, load_queue_size=64,
            prf_gpr_size=512, prf_fpr_size=256,
        ))
    elif config_name == "small":
        return Config(width=4, backend=Backend.OutOfOrder(
            rob_size=16, issue_queue_size=8,
            store_buffer_size=4, load_queue_size=8,
            prf_gpr_size=64, prf_fpr_size=64,
        ))
    elif config_name == "inorder":
        return Config(width=4, backend=Backend.InOrder())
    elif config_name == "linux":
        return Config(
            width=8,
            branch_predictor=BranchPredictor.TAGE(
                num_banks=8, table_size=8192, loop_table_size=1024,
                reset_interval=500_000,
                history_lengths=[5, 11, 22, 44, 89, 178, 356, 712],
                tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
            ),
            btb_size=16384, btb_ways=8, ras_size=128,
            backend=Backend.OutOfOrder(
                rob_size=256, store_buffer_size=64, issue_queue_size=96,
                load_queue_size=64, load_ports=4, store_ports=2,
                prf_gpr_size=512, prf_fpr_size=256,
                fu_config=Fu([
                    Fu.IntAlu(count=6, latency=1),
                    Fu.IntMul(count=2, latency=3),
                    Fu.IntDiv(count=2, latency=20),
                    Fu.FpAdd(count=4, latency=4),
                    Fu.FpMul(count=4, latency=5),
                    Fu.FpFma(count=4, latency=5),
                    Fu.FpDivSqrt(count=2, latency=21),
                    Fu.Branch(count=4, latency=1),
                    Fu.Mem(count=4, latency=1),
                ]),
            ),
            l1i=Cache(size="64KB", line="64B", ways=8,
                      policy=ReplacementPolicy.PLRU(), latency=1,
                      prefetcher=Prefetcher.NextLine(degree=4), mshr_count=8),
            l1d=Cache(size="64KB", line="64B", ways=8,
                      policy=ReplacementPolicy.PLRU(), latency=1,
                      prefetcher=Prefetcher.Stride(degree=4, table_size=256),
                      mshr_count=16),
            l2=Cache(size="2MB", line="64B", ways=16,
                     policy=ReplacementPolicy.PLRU(), latency=8,
                     prefetcher=Prefetcher.Stream(degree=8), mshr_count=32),
            l3=Cache(size="16MB", line="64B", ways=16,
                     policy=ReplacementPolicy.PLRU(), latency=24,
                     prefetcher=Prefetcher.Tagged(degree=4), mshr_count=64),
            inclusion_policy=Cache.Inclusive(),
            wcb_entries=16,
            memory_controller=MemoryController.Simple(),
        )
    else:
        return Config(width=4, backend=Backend.OutOfOrder())


def compare_signatures(spike_sig, rvsim_sig):
    """Compare two signature dicts. Returns list of (reg, spike_val, rvsim_val) mismatches."""
    if spike_sig is None or rvsim_sig is None:
        return None  # Can't compare
    mismatches = []
    for reg in SIG_REGS:
        sv = spike_sig.get(reg)
        rv = rvsim_sig.get(reg)
        if sv is not None and rv is not None and sv != rv:
            mismatches.append((reg, sv, rv))
    return mismatches


def format_mismatch(reg, spike_val, rvsim_val):
    """Format a register mismatch for display."""
    name = ABI_NAMES.get(reg, f"x{reg}")
    return (
        f"  x{reg:2d} ({name:>3s}): "
        f"spike=0x{spike_val:016x}  rvsim=0x{rvsim_val:016x}  "
        f"diff=0x{spike_val ^ rvsim_val:016x}"
    )


def main():
    ap = argparse.ArgumentParser(description="Deep fuzzing: signature comparison")
    ap.add_argument("--seed", type=int, default=0, help="Starting seed")
    ap.add_argument("--count", type=int, default=100, help="Number of tests")
    ap.add_argument("--length", type=int, default=500, help="Instructions per test")
    ap.add_argument("--skip-generate", action="store_true", help="Reuse existing .S files")
    ap.add_argument("--keep-failing", action="store_true", help="Keep failing ELFs")
    ap.add_argument(
        "--config", default="default",
        choices=["default", "wide", "small", "inorder", "linux"],
        help="rvsim pipeline config to test",
    )
    ap.add_argument("--mem-pct", type=int, default=30)
    ap.add_argument("--branch-pct", type=int, default=15)
    ap.add_argument("--verbose", "-v", action="store_true", help="Show all mismatches")
    args = ap.parse_args()

    os.makedirs(BUILD_DIR, exist_ok=True)
    os.makedirs(GEN_DIR, exist_ok=True)
    if args.keep_failing:
        os.makedirs(FAIL_DIR, exist_ok=True)

    # Step 1: Generate
    if not args.skip_generate:
        print(f"[fuzz] Generating {args.count} tests (seed={args.seed}, length={args.length})...")
        sys.path.insert(0, TORTURE_DIR)
        from generate import TortureGenerator
        for i in range(args.count):
            seed = args.seed + i
            gen = TortureGenerator(
                seed=seed, length=args.length,
                mem_ops_pct=args.mem_pct, branch_pct=args.branch_pct,
            )
            code = gen.generate()
            path = os.path.join(GEN_DIR, f"torture_{seed:06d}.S")
            with open(path, "w") as f:
                f.write(code)

    # Collect .S files
    src_files = sorted(f for f in os.listdir(GEN_DIR) if f.endswith(".S"))
    if not src_files:
        print("[fuzz] No .S files found in", GEN_DIR)
        return 1

    # Step 2: Assemble
    print(f"[fuzz] Building {len(src_files)} tests...")
    elfs = []
    build_fail = 0
    for src_name in src_files:
        src_path = os.path.join(GEN_DIR, src_name)
        elf_name = src_name.replace(".S", "")
        elf_path = os.path.join(BUILD_DIR, elf_name)
        if assemble_test(src_path, elf_path):
            elfs.append((elf_name, elf_path))
        else:
            build_fail += 1

    if build_fail:
        print(f"[fuzz] {build_fail} tests failed to assemble")

    print(f"[fuzz] Running {len(elfs)} tests (config={args.config})...")
    print(f"[fuzz] Comparing signatures: Spike vs rvsim")
    print()

    # Step 3: Run and compare
    passed = 0
    exit_code_fail = []
    sig_mismatch = []
    spike_err = []
    no_sig = 0

    for i, (name, elf_path) in enumerate(elfs):
        # Run Spike
        try:
            spike_rc, spike_sig = run_spike_signature(elf_path)
        except Exception as e:
            spike_err.append((name, str(e)))
            continue

        if spike_rc != 0:
            spike_err.append((name, f"exit={spike_rc}"))
            continue

        # Run rvsim
        rvsim_rc, rvsim_sig = run_rvsim_signature(elf_path, args.config)

        # Compare exit codes first
        if rvsim_rc != 0:
            exit_code_fail.append((name, spike_rc, rvsim_rc))
            if args.keep_failing:
                _save_failure(name, elf_path)
            continue

        # Compare signatures
        mismatches = compare_signatures(spike_sig, rvsim_sig)
        if mismatches is None:
            no_sig += 1
            passed += 1  # Can't compare, but exit codes matched
        elif len(mismatches) == 0:
            passed += 1
        else:
            sig_mismatch.append((name, mismatches))
            if args.verbose or len(sig_mismatch) <= 5:
                print(f"  MISMATCH: {name} ({len(mismatches)} register(s) differ)")
                for reg, sv, rv in mismatches[:8]:
                    print(format_mismatch(reg, sv, rv))
                if len(mismatches) > 8:
                    print(f"    ... and {len(mismatches) - 8} more")
            if args.keep_failing:
                _save_failure(name, elf_path)

        # Progress
        total = i + 1
        if total % 10 == 0 or total == len(elfs):
            print(
                f"  [{total:4d}/{len(elfs)}] "
                f"pass={passed} exit_fail={len(exit_code_fail)} "
                f"sig_mismatch={len(sig_mismatch)} spike_err={len(spike_err)}"
            )

    # Summary
    print()
    print("=" * 72)
    print(
        f"FUZZ RESULTS: {passed} passed, "
        f"{len(exit_code_fail)} exit-code failures, "
        f"{len(sig_mismatch)} SIGNATURE MISMATCHES, "
        f"{len(spike_err)} spike errors, "
        f"{build_fail} build errors"
    )
    if no_sig > 0:
        print(f"  ({no_sig} tests had no signature data for comparison)")
    print(f"Config: {args.config}")
    print("=" * 72)

    if sig_mismatch:
        print(f"\n=== {len(sig_mismatch)} Signature Mismatches (SILENT DATA CORRUPTION) ===")
        for name, mismatches in sig_mismatch:
            regs_str = ", ".join(
                f"x{r}({ABI_NAMES.get(r, '?')})" for r, _, _ in mismatches[:5]
            )
            print(f"  {name}: {len(mismatches)} regs differ: {regs_str}")
        if args.keep_failing:
            print(f"\nFailing ELFs + sources saved to: {FAIL_DIR}")

    if exit_code_fail:
        print(f"\n=== {len(exit_code_fail)} Exit Code Failures ===")
        for name, src, rvc in exit_code_fail[:10]:
            print(f"  {name}  (spike={src}, rvsim={rvc})")

    return 1 if (sig_mismatch or exit_code_fail) else 0


def _save_failure(name, elf_path):
    """Save failing ELF and source to failures dir."""
    shutil.copy2(elf_path, os.path.join(FAIL_DIR, name))
    src_path = os.path.join(GEN_DIR, name + ".S")
    if os.path.exists(src_path):
        shutil.copy2(src_path, os.path.join(FAIL_DIR, name + ".S"))


if __name__ == "__main__":
    sys.exit(main())
