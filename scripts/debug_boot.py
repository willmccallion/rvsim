#!/usr/bin/env python3
"""Debug script for Linux boot."""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from rvsim import (
    BranchPredictor,
    Cache,
    Config,
    Prefetcher,
    ReplacementPolicy,
    Simulator,
)


def optimized_config():
    return Config(
        width=4,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=4,
            table_size=4096,
            loop_table_size=512,
            reset_interval=2000,
            history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10],
        ),
        btb_size=8192,
        ras_size=64,
        ram_base=0x80000000,
        uart_base=0x10000000,
        disk_base=0x10001000,
        clint_base=0x02000000,
        syscon_base=0x00100000,
        kernel_offset=0x200000,
        bus_width=8,
        bus_latency=1,
        clint_divider=1,
        ram_size="256MB",
        memory_controller=None,
        tlb_size=128,
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


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    linux_dir = os.path.join(root, "software", "linux")
    out_dir = os.path.join(linux_dir, "output")
    image_path = os.path.join(out_dir, "Image")
    disk_path = os.path.join(out_dir, "disk.img")
    dtb_path = os.path.join(linux_dir, "system.dtb")

    os.chdir(root)

    sim = Simulator().config(optimized_config()).kernel(image_path).disk(disk_path)
    if os.path.isfile(dtb_path):
        sim.dtb(dtb_path)

    cpu = sim.build()

    # Run 200K instructions in chunks, watching for progress
    print("=== Running 200K instructions ===")
    chunk = 20000
    for c in range(10):
        for i in range(chunk):
            inst = cpu.step()
            if inst is None:
                print(f"  Exited at step {c * chunk + i}")
                return
        step_num = (c + 1) * chunk
        pc = cpu.pc
        print(f"  Step {step_num:6d}: PC={pc:#x}")

    print("Completed 200K steps.")


if __name__ == "__main__":
    main()
