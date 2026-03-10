#!/usr/bin/env python3
"""Boot Linux with in-order backend to check if crash is O3-specific."""
import os
import sys

from rvsim import Backend, Cache, Config, MemoryController, Prefetcher, ReplacementPolicy, Simulator


def repo_root():
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def main():
    root = repo_root()
    linux_dir = os.path.join(root, "software", "linux")
    image = os.path.join(linux_dir, "output", "Image")
    disk = os.path.join(linux_dir, "output", "disk.img")
    dtb = os.path.join(linux_dir, "system.dtb")

    cfg = Config(
        width=1,
        backend=Backend.InOrder(),
        l1i=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.NextLine(degree=4), mshr_count=8),
        l1d=Cache(size="64KB", line="64B", ways=8, policy=ReplacementPolicy.PLRU(),
                  latency=1, prefetcher=Prefetcher.Stride(degree=4, table_size=256), mshr_count=16),
        l2=Cache(size="2MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(),
                 latency=8, prefetcher=Prefetcher.Stream(degree=8), mshr_count=32),
        l3=Cache(size="16MB", line="64B", ways=16, policy=ReplacementPolicy.PLRU(),
                 latency=24, prefetcher=Prefetcher.Tagged(degree=4), mshr_count=64),
        inclusion_policy=Cache.Inclusive(),
        wcb_entries=16,
        ram_size="256MB", tlb_size=256, l2_tlb_size=2048, l2_tlb_ways=8, l2_tlb_latency=3,
        memory_controller=MemoryController.Simple(),
        ram_base=0x80000000, uart_base=0x10000000, disk_base=0x10001000,
        clint_base=0x02000000, syscon_base=0x00100000, kernel_offset=0x200000,
        bus_width=8, bus_latency=1, clint_divider=1,
    )

    sim = Simulator().config(cfg).kernel(image).disk(disk)
    if os.path.isfile(dtb):
        sim.dtb(dtb)

    try:
        rc = sim.run(limit=100_000_000_000)
        print(f"Exited with code {rc}", file=sys.stderr)
        return rc
    except Exception as e:
        print(f"Simulation ended: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
