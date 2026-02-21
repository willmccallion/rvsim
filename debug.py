#!/usr/bin/env python3
"""Debug: Trace the exact panic cause.

The crash at ~46.8M cycles / 19.7M retired — kernel panic detected.
scause=0x8000000000000005 (supervisor timer interrupt)
sepc=0xffffffff8005f752

Strategy: Run to 44M cycles, save, then step and capture the last ~500
instructions before the stall, including full register diffs.
"""
import os, sys
from rvsim import (BranchPredictor, Cache, Config,
                   Prefetcher, ReplacementPolicy, Simulator, reg)

LOG = "/tmp/debug_boot.log"

def hex64(v): return f"0x{v:016x}"

def make_config(width):
    return Config(
        width=width,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=4, table_size=4096, loop_table_size=512,
            reset_interval=2000, history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10]),
        btb_size=8192, ras_size=64,
        ram_base=0x80000000,
        uart_base=0x10000000, disk_base=0x10001000,
        clint_base=0x02000000, syscon_base=0x00100000,
        kernel_offset=0x200000, bus_width=8, bus_latency=1,
        clint_divider=1, ram_size="256MB", memory_controller=None,
        tlb_size=128,
        l1i=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.NextLine(degree=2)),
        l1d=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
        l2=Cache(size="1MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(),
                 latency=8, prefetcher=Prefetcher.NextLine(degree=1)),
        l3=Cache(size="8MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(), latency=28))

REG_NAMES = [
    "zero","ra","sp","gp","tp","t0","t1","t2",
    "s0","s1","a0","a1","a2","a3","a4","a5",
    "a6","a7","s2","s3","s4","s5","s6","s7",
    "s8","s9","s10","s11","t3","t4","t5","t6"
]

def dump_regs(cpu):
    return {i: cpu.regs[i] for i in range(32)}

def diff_regs(before, after):
    changes = []
    for i in range(1, 32):  # skip x0
        if before[i] != after[i]:
            changes.append(f"{REG_NAMES[i]}={hex64(after[i])}")
    return ", ".join(changes) if changes else ""

def main():
    root = os.path.dirname(os.path.abspath(__file__))
    out = os.path.join(root, "software", "linux", "output")
    dtb = os.path.join(root, "software", "linux", "system.dtb")
    f = open(LOG, "w")

    PERCPU_INIT_RWSEM = 0xffffffff8005f742

    cfg = make_config(4)
    sim = Simulator().config(cfg).kernel(os.path.join(out, "Image")).disk(os.path.join(out, "disk.img"))
    if os.path.isfile(dtb):
        sim = sim.dtb(dtb)
    cpu = sim.build()

    # Run to 44M cycles
    print("Running to 44M cycles...")
    cpu.run(limit=44_000_000)
    ret0 = cpu.stats.get("instructions_retired", 0)
    cyc0 = cpu.stats["cycles"]
    print(f"  At {cyc0:,} cycles, {ret0:,} retired")
    f.write(f"At {cyc0:,} cycles, {ret0:,} retired\n\n")

    ckpt = "/tmp/rvsim_checkpoint.bin"
    cpu.save(ckpt)

    # Step and record every instruction with register diffs
    print("Stepping to capture all instructions before panic...")
    f.write("Stepping to capture all instructions before panic...\n\n")
    f.flush()

    history = []  # (pc, asm, reg_changes)
    prev_regs = dump_regs(cpu)

    for i in range(5_000_000):
        inst = cpu.step()
        if not inst:
            # Stall/exit — dump the last 500 instructions
            f.write(f"STALL/EXIT at step {i}, cycle {cpu.stats['cycles']:,}, "
                    f"retired {cpu.stats.get('instructions_retired', 0):,}\n\n")
            f.write(f"Last {min(len(history), 500)} instructions before stall:\n\n")
            for pc, asm, changes in history[-500:]:
                if changes:
                    f.write(f"  {hex64(pc)} {asm:35s} [{changes}]\n")
                else:
                    f.write(f"  {hex64(pc)} {asm}\n")

            f.write(f"\nFinal registers:\n")
            for r in range(32):
                f.write(f"  {REG_NAMES[r]:4s} = {hex64(cpu.regs[r])}\n")
            f.write(f"\nCSRs:\n")
            f.write(f"  scause  = {hex64(cpu.csrs['scause'])}\n")
            f.write(f"  sepc    = {hex64(cpu.csrs['sepc'])}\n")
            f.write(f"  stval   = {hex64(cpu.csrs['stval'])}\n")
            f.write(f"  sstatus = {hex64(cpu.csrs['sstatus'])}\n")
            f.write(f"  mcause  = {hex64(cpu.csrs['mcause'])}\n")
            f.write(f"  mepc    = {hex64(cpu.csrs['mepc'])}\n")
            f.write(f"  mstatus = {hex64(cpu.csrs['mstatus'])}\n")
            f.flush()
            print(f"  Stall at step {i}, cycle {cpu.stats['cycles']:,}")
            break

        new_regs = dump_regs(cpu)
        changes = diff_regs(prev_regs, new_regs)
        history.append((inst.pc, inst.asm, changes))
        prev_regs = new_regs

        # Also flag interesting PCs
        if inst.pc == PERCPU_INIT_RWSEM:
            history[-1] = (inst.pc, inst.asm + " <== __percpu_init_rwsem ENTRY", changes)

        if i % 500_000 == 0 and i > 0:
            print(f"    {i/1e6:.1f}M steps")
            f.flush()

    f.write("\n=== Done ===\n")
    f.close()
    print(f"\nFull log at {LOG}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
