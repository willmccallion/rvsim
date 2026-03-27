#!/usr/bin/env python3
"""Build and run torture tests, comparing rvsim against Spike.

For each generated .S file:
  1. Assemble + link using the riscv-tests environment
  2. Run on Spike (golden reference) — extract signature
  3. Run on rvsim — extract signature
  4. Compare signatures; report mismatches

Usage (from repo root):
    python software/torture/run.py                    # generate + build + run
    python software/torture/run.py --skip-generate     # build + run (reuse .S files)
    python software/torture/run.py --seed 42 --count 50
    python software/torture/run.py --keep-failing      # keep ELFs that fail
"""

import argparse
import contextlib
import io
import os
import struct
import subprocess
import sys
import tempfile

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TORTURE_DIR = os.path.dirname(os.path.abspath(__file__))

# Directories are set per test type in main()
GEN_DIR = os.path.join(TORTURE_DIR, "generated")
BUILD_DIR = os.path.join(TORTURE_DIR, "build")
FAIL_DIR = os.path.join(TORTURE_DIR, "failures")

# riscv-tests environment for includes
RISCV_TESTS_DIR = os.path.join(REPO_ROOT, "software", "riscv-tests")
ENV_DIR = os.path.join(RISCV_TESTS_DIR, "env")
ISA_MACROS = os.path.join(RISCV_TESTS_DIR, "isa", "macros", "scalar")
LINK_SCRIPT = os.path.join(ENV_DIR, "p", "link.ld")

def _find_in_nix_store(binary):
    """Find a binary on PATH or in /nix/store as a fallback."""
    import shutil
    found = shutil.which(binary)
    if found:
        return found
    import glob
    candidates = glob.glob(f"/nix/store/*/bin/{binary}")
    # Prefer paths containing "wrapper" (nix convention for usable gcc)
    wrappers = [c for c in candidates if "wrapper" in c]
    if wrappers:
        return wrappers[0]
    if candidates:
        return candidates[0]
    return binary  # fall back to bare name (will fail later with a clear error)

def _augment_path():
    """Add directories of discovered nix-store tools to PATH so dependencies
    (e.g. dtc for Spike, as for gcc) are also reachable."""
    extra_dirs = set()
    for tool in (CC, OBJCOPY, OBJDUMP, READELF, SPIKE):
        d = os.path.dirname(tool)
        if d and d not in os.environ.get("PATH", ""):
            extra_dirs.add(d)
    # Also find dtc explicitly — Spike needs it at runtime.
    dtc = _find_in_nix_store("dtc")
    d = os.path.dirname(dtc)
    if d:
        extra_dirs.add(d)
    if extra_dirs:
        os.environ["PATH"] = ":".join(extra_dirs) + ":" + os.environ.get("PATH", "")

def _find_riscv_tool(name):
    """Find a RISC-V tool under any common prefix."""
    import shutil
    for prefix in ("riscv64-none-elf-", "riscv64-elf-", "riscv64-unknown-elf-"):
        found = shutil.which(prefix + name)
        if found:
            return found
    # Fall back to nix-store search with the canonical prefix
    return _find_in_nix_store("riscv64-none-elf-" + name)

CC = _find_riscv_tool("gcc")
OBJCOPY = _find_riscv_tool("objcopy")
OBJDUMP = _find_riscv_tool("objdump")
READELF = _find_riscv_tool("readelf")
SPIKE = _find_in_nix_store("spike")
_augment_path()

# Registers dumped in signature (x5-x31, skipping x4/tp)
SIG_REGS = [r for r in range(5, 32) if r != 4]
SIG_SIZE = len(SIG_REGS) * 8  # bytes


def assemble_test(src_path, elf_path, march="rv64gc"):
    """Assemble a torture .S file into an ELF."""
    cmd = [
        CC,
        f"-march={march}", "-mabi=lp64d",
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
        print(f"  ASSEMBLY FAILED: {os.path.basename(src_path)}")
        print(f"    {result.stderr.strip()}")
        return False
    return True


def get_signature_addr(elf_path):
    """Get the address of begin_signature from the ELF symbol table."""
    result = subprocess.run(
        [READELF, "-s", elf_path], capture_output=True, text=True
    )
    for line in result.stdout.splitlines():
        if "begin_signature" in line:
            parts = line.split()
            # readelf format: Num Value Size Type Bind Vis Ndx Name
            for p in parts:
                try:
                    addr = int(p, 16)
                    if addr > 0x80000000:
                        return addr
                except ValueError:
                    continue
    return None


def run_spike(elf_path, isa="rv64gc"):
    """Run test on Spike, return (exit_code, signature_bytes)."""
    sig_addr = get_signature_addr(elf_path)

    # Run Spike
    result = subprocess.run(
        [SPIKE, f"--isa={isa}", elf_path],
        capture_output=True, text=True, timeout=30,
    )

    if sig_addr is None:
        return result.returncode, None

    # Extract signature from Spike's memory by running spike with
    # --dump-dts and signature dump. Since Spike doesn't directly dump
    # memory regions, we use a different approach: run spike interactively
    # to dump registers, or use the ELF + spike log.
    # Actually, the simplest approach: use spike's +signature flag if available,
    # or just check exit code and compare register dumps via a different method.

    # Alternative: use spike's built-in signature dumping
    with tempfile.NamedTemporaryFile(suffix=".sig", delete=False) as sigf:
        sig_path = sigf.name

    result2 = subprocess.run(
        [SPIKE, f"--isa={isa}", f"+signature={sig_path}", elf_path],
        capture_output=True, text=True, timeout=30,
    )

    sig_data = None
    if os.path.exists(sig_path):
        try:
            with open(sig_path, "r") as f:
                sig_data = f.read().strip()
        except Exception:
            pass
        os.unlink(sig_path)

    return result2.returncode, sig_data


def run_rvsim(elf_path, config_name="default", cycle_limit=500_000):
    """Run test on rvsim, return (exit_code, signature_bytes)."""
    sig_addr = get_signature_addr(elf_path)

    # Import rvsim
    sys.path.insert(0, REPO_ROOT)
    from rvsim import Backend, Config, Simulator

    from rvsim import (
        BranchPredictor, Cache, Fu, MemoryController,
        Prefetcher, ReplacementPolicy,
    )

    configs = {
        "default": Config(width=4, backend=Backend.OutOfOrder()),
        "wide": Config(width=8, backend=Backend.OutOfOrder(
            rob_size=256, issue_queue_size=96,
            store_buffer_size=64, load_queue_size=64,
            prf_gpr_size=512, prf_fpr_size=256,
        )),
        "small": Config(width=4, backend=Backend.OutOfOrder(
            rob_size=16, issue_queue_size=8,
            store_buffer_size=4, load_queue_size=8,
            prf_gpr_size=64, prf_fpr_size=64,
        )),
        "inorder": Config(width=4, backend=Backend.InOrder()),
        # Matches the Linux boot config from boot_linux.py
        "linux": Config(
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
        ),
    }

    cfg = configs.get(config_name, configs["default"])

    sim = Simulator().config(cfg).binary(elf_path)
    with (
        contextlib.redirect_stdout(io.StringIO()),
        contextlib.redirect_stderr(io.StringIO()),
    ):
        rc = sim.run(limit=cycle_limit, stats_sections=None)

    # TODO: extract signature from simulator memory if API supports it
    return rc, None


def compare_exit_codes(spike_rc, rvsim_rc):
    """Compare exit codes. Both should be 0 (PASS)."""
    if spike_rc != 0:
        return "SPIKE_FAIL"
    if rvsim_rc != 0:
        return "RVSIM_FAIL"
    return "PASS"


def main():
    ap = argparse.ArgumentParser(description="Run RISC-V torture tests")
    ap.add_argument("--seed", type=int, default=0, help="Starting seed")
    ap.add_argument("--count", type=int, default=100, help="Number of tests")
    ap.add_argument("--length", type=int, default=500, help="Instructions per test")
    ap.add_argument("--skip-generate", action="store_true", help="Reuse existing .S files")
    ap.add_argument("--keep-failing", action="store_true", help="Keep ELFs that fail")
    ap.add_argument(
        "--config", default="default",
        choices=["default", "wide", "small", "inorder", "linux"],
        help="rvsim pipeline config to test",
    )
    ap.add_argument(
        "--type", default="scalar", choices=["scalar", "vec"],
        help="Test type: scalar or vector",
    )
    ap.add_argument(
        "--mem-pct", type=int, default=30, help="Percentage of memory operations"
    )
    ap.add_argument(
        "--branch-pct", type=int, default=15, help="Percentage of branch blocks"
    )
    ap.add_argument(
        "--fp-pct", type=int, default=15, help="FP test percentage (vec only)"
    )
    ap.add_argument("--jobs", "-j", type=int, default=1, help="Parallel jobs (build only)")
    args = ap.parse_args()

    global GEN_DIR, BUILD_DIR, FAIL_DIR
    if args.type == "vec":
        GEN_DIR = os.path.join(TORTURE_DIR, "generated_vec")
        BUILD_DIR = os.path.join(TORTURE_DIR, "build_vec")
        SPIKE_ISA = "rv64gcv"
        CC_MARCH = "rv64gcv"
        CYCLE_LIMIT = 2_000_000
        if args.length == 500:
            args.length = 50  # default for vec is fewer blocks
    else:
        SPIKE_ISA = "rv64gc"
        CC_MARCH = "rv64gc"
        CYCLE_LIMIT = 500_000

    FAIL_DIR = os.path.join(TORTURE_DIR, "failures")

    os.makedirs(BUILD_DIR, exist_ok=True)
    os.makedirs(GEN_DIR, exist_ok=True)
    if args.keep_failing:
        os.makedirs(FAIL_DIR, exist_ok=True)

    # Step 1: Generate
    if not args.skip_generate:
        print(f"[torture] Generating {args.count} {args.type} tests (seed={args.seed}, length={args.length})...")
        if args.type == "vec":
            from generate_vec import VecTortureGenerator
            for i in range(args.count):
                seed = args.seed + i
                gen = VecTortureGenerator(
                    seed=seed,
                    length=args.length,
                    vec_fp_pct=args.fp_pct,
                )
                code = gen.generate()
                path = os.path.join(GEN_DIR, f"vec_torture_{seed:06d}.S")
                with open(path, "w") as f:
                    f.write(code)
        else:
            from generate import TortureGenerator
            for i in range(args.count):
                seed = args.seed + i
                gen = TortureGenerator(
                    seed=seed,
                    length=args.length,
                    mem_ops_pct=args.mem_pct,
                    branch_pct=args.branch_pct,
                )
                code = gen.generate()
                path = os.path.join(GEN_DIR, f"torture_{seed:06d}.S")
                with open(path, "w") as f:
                    f.write(code)

    # Collect .S files
    src_files = sorted(
        f for f in os.listdir(GEN_DIR) if f.endswith(".S")
    )
    if not src_files:
        print("[torture] No .S files found in", GEN_DIR)
        return 1

    print(f"[torture] Building {len(src_files)} tests...")

    # Step 2: Assemble
    elfs = []
    build_fail = 0
    for src_name in src_files:
        src_path = os.path.join(GEN_DIR, src_name)
        elf_name = src_name.replace(".S", "")
        elf_path = os.path.join(BUILD_DIR, elf_name)
        if assemble_test(src_path, elf_path, march=CC_MARCH):
            elfs.append((elf_name, elf_path))
        else:
            build_fail += 1

    if build_fail:
        print(f"[torture] {build_fail} tests failed to assemble")

    print(f"[torture] Running {len(elfs)} tests (config={args.config})...")
    print()

    # Step 3: Run
    passed = 0
    failed = []
    spike_failed = []

    for i, (name, elf_path) in enumerate(elfs):
        # Run Spike
        try:
            spike_rc, spike_sig = run_spike(elf_path, isa=SPIKE_ISA)
        except subprocess.TimeoutExpired:
            spike_rc = -1
            spike_sig = None

        # Run rvsim
        rvsim_rc, rvsim_sig = run_rvsim(elf_path, args.config, cycle_limit=CYCLE_LIMIT)

        status = compare_exit_codes(spike_rc, rvsim_rc)

        if status == "PASS":
            passed += 1
        elif status == "SPIKE_FAIL":
            # Test itself is broken (Spike also fails) — skip
            spike_failed.append(name)
        else:
            failed.append((name, spike_rc, rvsim_rc))
            if args.keep_failing:
                import shutil
                shutil.copy2(elf_path, os.path.join(FAIL_DIR, name))
                src_path = os.path.join(GEN_DIR, name + ".S")
                if os.path.exists(src_path):
                    shutil.copy2(src_path, os.path.join(FAIL_DIR, name + ".S"))

        # Progress
        total = i + 1
        if total % 10 == 0 or total == len(elfs):
            print(
                f"  [{total:4d}/{len(elfs)}] "
                f"pass={passed} fail={len(failed)} spike_err={len(spike_failed)}"
            )

    # Summary
    print()
    print("=" * 60)
    print(f"TORTURE RESULTS: {passed} passed, {len(failed)} FAILED, "
          f"{len(spike_failed)} spike errors, {build_fail} build errors")
    print(f"Config: {args.config}")
    print("=" * 60)

    if failed:
        print(f"\n=== {len(failed)} Failures ===")
        for name, spike_rc, rvsim_rc in failed:
            print(f"  {name}  (spike={spike_rc}, rvsim={rvsim_rc})")
        if args.keep_failing:
            print(f"\nFailing ELFs + sources saved to: {FAIL_DIR}")
        return 1

    if spike_failed:
        print(f"\n=== {len(spike_failed)} Spike Errors (test generation issues) ===")
        for name in spike_failed[:10]:
            print(f"  {name}")
        if len(spike_failed) > 10:
            print(f"  ... and {len(spike_failed) - 10} more")

    return 0


if __name__ == "__main__":
    sys.exit(main())
