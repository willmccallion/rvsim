#!/usr/bin/env python3
"""Build and run self-checking memory torture tests on rvsim.

These tests are self-checking — they trap (RVTEST_FAIL) on mismatch.
No Spike needed. Exit code 0 = PASS, non-zero = FAIL.

Usage:
    python run_memcheck.py                          # generate + build + run (default config)
    python run_memcheck.py --config linux --count 200
    python run_memcheck.py --skip-generate          # reuse existing .S files
    python run_memcheck.py --all-configs             # run across all configs
"""

import argparse
import contextlib
import io
import os
import subprocess
import sys
import traceback

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


def assemble(src_path, elf_path):
    cmd = [
        CC, "-march=rv64gc", "-mabi=lp64d",
        "-static", "-mcmodel=medany",
        "-fvisibility=hidden", "-nostdlib", "-nostartfiles",
        f"-I{os.path.join(ENV_DIR, 'p')}",
        f"-I{ENV_DIR}",
        f"-I{ISA_MACROS}",
        f"-T{LINK_SCRIPT}",
        "-o", elf_path, src_path,
    ]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"  BUILD FAIL: {os.path.basename(src_path)}: {result.stderr.strip()}")
        return False
    return True


def run_rvsim(elf_path, config_name):
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
    stderr_capture = io.StringIO()
    with (
        contextlib.redirect_stdout(io.StringIO()),
        contextlib.redirect_stderr(stderr_capture),
    ):
        rc = sim.run(limit=2_000_000, stats_sections=None)
    return rc, stderr_capture.getvalue()


def main():
    ap = argparse.ArgumentParser(description="Run self-checking memory torture tests")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--length", type=int, default=300)
    ap.add_argument("--skip-generate", action="store_true")
    ap.add_argument("--config", default="default",
                    choices=["default", "wide", "small", "inorder", "linux"])
    ap.add_argument("--all-configs", action="store_true")
    ap.add_argument("--keep-failing", action="store_true")
    ap.add_argument("--generator", default="memcheck",
                    choices=["memcheck", "memstress", "both"],
                    help="Which generator to use")
    args = ap.parse_args()

    os.makedirs(BUILD_DIR, exist_ok=True)
    os.makedirs(GEN_DIR, exist_ok=True)
    if args.keep_failing:
        os.makedirs(FAIL_DIR, exist_ok=True)

    # Generate
    if not args.skip_generate:
        sys.path.insert(0, TORTURE_DIR)
        gens_to_run = []
        if args.generator in ("memcheck", "both"):
            gens_to_run.append(("memcheck", "gen_memcheck", "MemCheckGenerator"))
        if args.generator in ("memstress", "both"):
            gens_to_run.append(("memstress", "gen_memstress", "MemStressGenerator"))

        for prefix, module_name, class_name in gens_to_run:
            print(f"[{prefix}] Generating {args.count} tests (seed={args.seed}, length={args.length})...")
            mod = __import__(module_name)
            GenClass = getattr(mod, class_name)
            for i in range(args.count):
                seed = args.seed + i
                gen = GenClass(seed=seed, length=args.length)
                code = gen.generate()
                path = os.path.join(GEN_DIR, f"{prefix}_{seed:06d}.S")
                with open(path, "w") as f:
                    f.write(code)

    # Collect .S files
    prefixes = []
    if args.generator in ("memcheck", "both"):
        prefixes.append("memcheck_")
    if args.generator in ("memstress", "both"):
        prefixes.append("memstress_")
    src_files = sorted(
        f for f in os.listdir(GEN_DIR)
        if f.endswith(".S") and any(f.startswith(p) for p in prefixes)
    )
    if not src_files:
        print("[memcheck] No memcheck .S files found")
        return 1

    # Assemble
    print(f"[memcheck] Assembling {len(src_files)} tests...")
    elfs = []
    build_fail = 0
    for src_name in src_files:
        src_path = os.path.join(GEN_DIR, src_name)
        elf_name = src_name.replace(".S", "")
        elf_path = os.path.join(BUILD_DIR, elf_name)
        if assemble(src_path, elf_path):
            elfs.append((elf_name, elf_path))
        else:
            build_fail += 1

    if build_fail:
        print(f"[memcheck] {build_fail} failed to assemble")

    configs_to_run = ["default", "wide", "small", "inorder", "linux"] if args.all_configs else [args.config]

    any_fail = False
    for config_name in configs_to_run:
        print(f"\n[memcheck] Running {len(elfs)} tests (config={config_name})...")
        passed = 0
        failed = []

        for i, (name, elf_path) in enumerate(elfs):
            try:
                rc, stderr = run_rvsim(elf_path, config_name)
            except Exception as e:
                rc = -1
                stderr = str(e)

            if rc == 0:
                passed += 1
            else:
                failed.append((name, rc, stderr))
                if args.keep_failing:
                    os.makedirs(FAIL_DIR, exist_ok=True)
                    import shutil
                    shutil.copy2(elf_path, os.path.join(FAIL_DIR, f"{name}.{config_name}"))
                    src_path = os.path.join(GEN_DIR, name + ".S")
                    if os.path.exists(src_path):
                        shutil.copy2(src_path, os.path.join(FAIL_DIR, f"{name}.{config_name}.S"))

            total = i + 1
            if total % 25 == 0 or total == len(elfs):
                print(f"  [{total:4d}/{len(elfs)}] pass={passed} FAIL={len(failed)}")

        print(f"\n{'='*60}")
        print(f"CONFIG={config_name}: {passed} passed, {len(failed)} FAILED (of {len(elfs)})")
        print(f"{'='*60}")

        if failed:
            any_fail = True
            print(f"\n  FAILURES:")
            for name, rc, stderr in failed[:20]:
                print(f"    {name} (rc={rc})")
                if stderr.strip():
                    for line in stderr.strip().splitlines()[:3]:
                        print(f"      {line}")
            if len(failed) > 20:
                print(f"    ... and {len(failed) - 20} more")

    return 1 if any_fail else 0


if __name__ == "__main__":
    sys.exit(main())
