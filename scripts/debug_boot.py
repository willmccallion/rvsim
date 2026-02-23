#!/usr/bin/env python3
"""Debug script for Linux boot hang after OpenSBI."""

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


def dump_pmp(cpu):
    """Dump PMP registers by reading pmpcfg0/pmpcfg2 and pmpaddr0-15."""
    # pmpcfg0 = CSR 0x3A0 (entries 0-7), pmpcfg2 = CSR 0x3A2 (entries 8-15)
    cfg0 = cpu.csrs[0x3A0]
    cfg2 = cpu.csrs[0x3A2]

    match_modes = {0: "OFF", 1: "TOR", 2: "NA4", 3: "NAPOT"}

    for i in range(16):
        if i < 8:
            cfg_byte = (cfg0 >> (i * 8)) & 0xFF
        else:
            cfg_byte = (cfg2 >> ((i - 8) * 8)) & 0xFF

        addr = cpu.csrs[0x3B0 + i]
        a_field = (cfg_byte >> 3) & 3
        mode = match_modes[a_field]
        r = "R" if cfg_byte & 1 else "-"
        w = "W" if cfg_byte & 2 else "-"
        x = "X" if cfg_byte & 4 else "-"
        l = "L" if cfg_byte & 0x80 else "-"

        if a_field == 0:
            range_str = "(disabled)"
        elif a_field == 1:  # TOR
            if i == 0:
                lo = 0
            else:
                prev_addr = cpu.csrs[0x3B0 + i - 1]
                lo = prev_addr << 2
            hi = addr << 2
            range_str = f"[{lo:#014x}, {hi:#014x})"
        elif a_field == 2:  # NA4
            base = addr << 2
            range_str = f"[{base:#014x}, {base + 4:#014x})"
        elif a_field == 3:  # NAPOT
            inv = ~addr & 0xFFFFFFFFFFFFFFFF
            trailing = 0
            tmp = inv
            while trailing < 64 and (tmp & 1) == 0:
                trailing += 1
                tmp >>= 1
            # Actually trailing_zeros of !addr gives trailing ones of addr
            # Wait, we need trailing ones of addr, which is trailing zeros of ~addr
            # Let me recompute: for NAPOT, count trailing ones of addr
            trailing_ones = 0
            tmp2 = addr
            while trailing_ones < 64 and (tmp2 & 1) == 1:
                trailing_ones += 1
                tmp2 >>= 1
            size_bits = trailing_ones + 3
            if size_bits >= 64:
                range_str = f"[0x0, 2^{size_bits}) (ALL MEMORY) pmpaddr={addr:#018x}"
            else:
                size = 1 << size_bits
                mask = size - 1
                base = (addr << 2) & ~mask & 0xFFFFFFFFFFFFFFFF
                range_str = f"[{base:#014x}, {base + size:#014x}) pmpaddr={addr:#018x}"
        else:
            range_str = ""

        if a_field != 0:
            print(f"  PMP{i:2d}: cfg={cfg_byte:#04x} {r}{w}{x}{l} mode={mode:5s} {range_str}")


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

    # Phase 1: Run through OpenSBI until we reach S-mode
    print("=== Phase 1: Running through OpenSBI (waiting for S-mode) ===")
    exit_code = cpu.run_until(privilege="S", limit=15_000_000, chunk=100_000)
    if exit_code is not None:
        print(f"  Exited with code {exit_code} before reaching S-mode!")
        return

    stats = cpu.stats
    print(f"  Reached S-mode at cycle {stats['cycles']}, {stats['instructions_retired']} insns")
    print(f"  Entry PC: {cpu.pc:#x}")
    print()

    print("=== PMP configuration at S-mode entry ===")
    dump_pmp(cpu)
    print()

    # Check: what does PMP say about 0x80200000?
    print("=== Checking address 0x80200000 against PMP (manual) ===")
    cfg0 = cpu.csrs[0x3A0]
    cfg2 = cpu.csrs[0x3A2]
    test_addr = 0x80200000
    for i in range(16):
        if i < 8:
            cfg_byte = (cfg0 >> (i * 8)) & 0xFF
        else:
            cfg_byte = (cfg2 >> ((i - 8) * 8)) & 0xFF
        a_field = (cfg_byte >> 3) & 3
        if a_field == 0:
            continue
        addr = cpu.csrs[0x3B0 + i]
        if a_field == 1:  # TOR
            prev_addr = cpu.csrs[0x3B0 + i - 1] << 2 if i > 0 else 0
            hi = addr << 2
            if test_addr >= prev_addr and test_addr < hi:
                x_bit = cfg_byte & 4
                print(f"  PMP{i} TOR matches! cfg={cfg_byte:#04x} X={'yes' if x_bit else 'NO'}")
                break
        elif a_field == 3:  # NAPOT
            trailing_ones = 0
            tmp = addr
            while trailing_ones < 64 and (tmp & 1) == 1:
                trailing_ones += 1
                tmp >>= 1
            size_bits = trailing_ones + 3
            if size_bits >= 64:
                size = 0  # overflow
                base = 0
                match = True
            else:
                size = 1 << size_bits
                mask = size - 1
                base = (addr << 2) & ~mask & 0xFFFFFFFFFFFFFFFF
                match = test_addr >= base and test_addr < base + size
            if match:
                x_bit = cfg_byte & 4
                print(f"  PMP{i} NAPOT matches! cfg={cfg_byte:#04x} X={'yes' if x_bit else 'NO'} size_bits={size_bits}")
                break
    else:
        print(f"  NO PMP entry matches 0x80200000! S-mode access will be DENIED.")
    print()

    # Step first few instructions to see what happens
    print("=== Stepping first 10 instructions from kernel entry ===")
    for i in range(10):
        inst = cpu.step()
        if inst is None:
            print(f"  Step {i}: <no commit>")
            continue
        print(f"  Step {i:3d}: PC={inst.pc:#010x} [{cpu.privilege}] {inst.asm}")
        if cpu.privilege == "M":
            print(f"    -> Trapped to M-mode! mcause={cpu.csrs['mcause']:#x} mepc={cpu.csrs['mepc']:#x}")
            break


if __name__ == "__main__":
    main()
