#!/usr/bin/env python3
"""
Debug script for diagnosing the Linux boot hang.

Focus: OpenSBI relocation loop is skipping instructions after ld at 0x80000080.
The bne at 0x80000086 is sometimes not executed, causing an infinite loop.

Usage:
    .venv/bin/rvsim --script scripts/setup/debug_boot_linux.py
"""

import os
import sys
import subprocess

_here = os.path.dirname(os.path.abspath(__file__))
_root = os.path.dirname(os.path.dirname(_here))
if _root not in sys.path:
    sys.path.insert(0, _root)

from rvsim import (
    Config,
    Cache,
    BranchPredictor,
    ReplacementPolicy,
    Prefetcher,
    disassemble,
)
from rvsim.objects import System, _create_cpu
from rvsim.isa import reg, reg_name


def repo_root() -> str:
    return _root


def optimized_config() -> Config:
    return Config(
        width=1,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=4,
            table_size=2048,
            loop_table_size=256,
            reset_interval=2000,
            history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10],
        ),
        btb_size=4096,
        ras_size=48,
        start_pc=0x80000000,
        ram_base=0x80000000,
        uart_base=0x10000000,
        disk_base=0x10001000,
        clint_base=0x02000000,
        syscon_base=0x00100000,
        kernel_offset=0x200000,
        bus_width=8,
        bus_latency=1,
        clint_divider=100,
        uart_to_stderr=True,
        ram_size=256 * 1024 * 1024,
        memory_controller=None,  # Simple
        tlb_size=64,
        l1i=Cache(
            size="64KB",
            line="64B",
            ways=8,
            policy=ReplacementPolicy.PLRU(),
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        l1d=Cache(
            size="64KB",
            line="64B",
            ways=8,
            policy=ReplacementPolicy.PLRU(),
            latency=1,
            prefetcher=Prefetcher.Stride(degree=2, table_size=128),
        ),
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=8,
            prefetcher=Prefetcher.NextLine(degree=1),
        ),
        l3=Cache(
            size="8MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=28,
        ),
    )


def setup_cpu():
    root = repo_root()
    linux_dir = os.path.join(root, "software", "linux")
    out_dir = os.path.join(linux_dir, "output")
    image_path = os.path.join(out_dir, "Image")
    disk_path = os.path.join(out_dir, "disk.img")
    dtb_path = os.path.join(linux_dir, "system.dtb")
    dts_path = os.path.join(linux_dir, "system.dts")

    for p in [image_path, disk_path]:
        if not os.path.exists(p):
            print(f"Error: {p} not found.", file=sys.stderr)
            sys.exit(1)

    if os.path.exists(dts_path):
        subprocess.run(
            ["dtc", "-I", "dts", "-O", "dtb", "-o", dtb_path, dts_path],
            cwd=linux_dir,
            capture_output=True,
        )

    os.chdir(root)
    cfg = optimized_config()
    sys_obj = System(ram_size=cfg.ram_size)
    sys_obj.instantiate(disk_image=disk_path, config=cfg)
    cpu = _create_cpu(sys_obj, config=cfg)
    cpu.load_kernel(image_path, dtb_path if os.path.exists(dtb_path) else None)
    return cpu


def main() -> int:
    cpu_wrapper = setup_cpu()
    # Use raw PyCpu for low-level access
    cpu = cpu_wrapper.raw
    out = sys.stderr

    print("=" * 72, file=out)
    print("DEBUG: Tracing OpenSBI relocation loop instruction-by-instruction", file=out)
    print("=" * 72, file=out)

    # Disassemble the relocation loop region
    print("\n  Memory dump of 0x80000078 - 0x800000e0:", file=out)
    addr = 0x80000078
    while addr < 0x800000E0:
        word = cpu.read_memory_u32(addr)
        is_compressed = (word & 0x3) != 0x3
        mnemonic = disassemble(word)
        if is_compressed:
            print(f"    {addr:#012x}:  {word & 0xFFFF:04x}      {mnemonic}", file=out)
            addr += 2
        else:
            print(f"    {addr:#012x}:  {word:08x}  {mnemonic}", file=out)
            addr += 4

    # Step through instructions, tracking every committed instruction
    print(
        "\n  Stepping through boot (showing all instructions in loop region)...",
        file=out,
    )
    print(f"  {'step':>5} {'cycles':>10} {'pc':>14}  {'asm':<45} regs", file=out)
    print(f"  {'-'*5} {'-'*10} {'-'*14}  {'-'*45} {'-'*40}", file=out)

    prev_pc = None
    loop_start = 0x80000078
    loop_end = 0x800000E0
    skip_detected = False

    for i in range(2000):
        result = cpu.step_instruction()
        if result is None:
            print(f"  [step {i}] sim exited", file=out)
            break

        pc, inst, asm = result
        cycles = cpu.get_stats().cycles

        # Always show instructions in the loop region
        in_loop = loop_start <= pc < loop_end

        if in_loop:
            t0 = cpu.read_register(reg.T0)
            t1 = cpu.read_register(reg.T1)
            t3 = cpu.read_register(reg.T3)
            t5 = cpu.read_register(reg.T5)

            # Detect skipped instructions
            marker = ""
            if prev_pc is not None and loop_start <= prev_pc < loop_end:
                prev_inst_word = cpu.read_memory_u32(prev_pc)
                prev_is_c = (prev_inst_word & 0x3) != 0x3
                prev_size = 2 if prev_is_c else 4
                expected_next = prev_pc + prev_size

                prev_asm = disassemble(prev_inst_word)
                is_branch_or_jump = any(
                    kw in prev_asm
                    for kw in [
                        "bne",
                        "beq",
                        "blt",
                        "bge",
                        "bltu",
                        "bgeu",
                        "jal",
                        "jalr",
                        "j ",
                    ]
                )

                if not is_branch_or_jump and pc != expected_next and pc != prev_pc:
                    marker = f" *** SKIP from {prev_pc:#x}+{prev_size}={expected_next:#x} ***"
                    skip_detected = True

            reg_str = f"t0={t0:#x} t1={t1:#x} t3={t3:#x} t5={t5:#x}"
            print(
                f"  {i:5} {cycles:10,} {pc:#014x}  {asm:<45} {reg_str}{marker}",
                file=out,
            )

        prev_pc = pc

        # After enough iterations, if we see t0 > t1, check if loop exits properly
        if in_loop and i > 100:
            t0 = cpu.read_register(reg.T0)
            t1 = cpu.read_register(reg.T1)
            if t0 >= t1 and pc == 0x800000D4:
                print(
                    f"\n  *** t0({t0:#x}) >= t1({t1:#x}) at blt — loop should exit! ***",
                    file=out,
                )

    if skip_detected:
        print("\n  *** INSTRUCTION SKIPS DETECTED — this is the bug! ***", file=out)
        print(
            "  The new pipeline is dropping instructions in the commit path.", file=out
        )
    else:
        print("\n  No instruction skips detected in 2000 steps.", file=out)

    return 0


if __name__ == "__main__":
    sys.exit(main())
