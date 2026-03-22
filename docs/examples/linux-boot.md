# Linux Boot

rvsim boots Linux 6.6 through OpenSBI to a BusyBox userspace shell on both the out-of-order and in-order backends.

## Prerequisites

Build the Linux kernel and root filesystem using Buildroot:

```bash
make -C software linux
```

This downloads and builds:

- **OpenSBI** firmware (loads at `0x8000_0000`)
- **Linux 6.6 kernel** (loads at `0x8020_0000`)
- **BusyBox rootfs** (ext2 image, mounted via VirtIO)

The build takes about 15-30 minutes on first run (cached after that).

## Booting

### Using the boot script

```bash
rvsim scripts/setup/boot_linux.py
```

This uses a pre-configured setup with realistic memory hierarchy parameters.

### Using the Python API

```python
from rvsim import (
    Simulator, Config, Cache, Backend, BranchPredictor,
    MemoryController, Prefetcher, ReplacementPolicy, Fu, MemDepPredictor,
)

config = Config(
    width=8,
    mem_dep_predictor=MemDepPredictor.StoreSet(),
    backend=Backend.OutOfOrder(rob_size=256),
    branch_predictor=BranchPredictor.ScLTage(
        num_banks=8, table_size=8192, loop_table_size=1024,
    ),
    ram_size="256MB",
    l1i=Cache("64KB", ways=8, latency=1,
              prefetcher=Prefetcher.NextLine(degree=4), mshr_count=8),
    l1d=Cache("64KB", ways=8, latency=1, mshr_count=16,
              prefetcher=Prefetcher.Stride(degree=4, table_size=256)),
    l2=Cache("2MB", ways=16, latency=8, mshr_count=32,
             prefetcher=Prefetcher.Stream(degree=8)),
    l3=Cache("16MB", ways=16, latency=24, mshr_count=64,
             prefetcher=Prefetcher.Tagged(degree=4)),
    memory_controller=MemoryController.DRAM(
        t_cas=14, t_ras=14, t_pre=14, row_miss_latency=120,
    ),
)

exit_code = (
    Simulator()
    .config(config)
    .kernel("software/linux/Image")
    .disk("software/linux/rootfs.ext2")
    .build()
    .run()
)
```

### Login

Once the kernel boots and init starts, you'll see a login prompt on the UART:

```
Welcome to Buildroot
buildroot login:
```

Login as `root` (no password). You'll get a BusyBox shell with standard POSIX utilities.

## In-Order Boot

The in-order backend also boots Linux, though significantly slower:

```python
config = Config(
    width=1,
    backend=Backend.InOrder(),
    branch_predictor=BranchPredictor.TAGE(),
    ram_size="256MB",
    l1d=Cache("32KB", ways=4, latency=1, mshr_count=16),
    l2=Cache("256KB", ways=8, latency=8, mshr_count=32),
    memory_controller=MemoryController.DRAM(t_cas=14, row_miss_latency=120),
)
```

## Boot Flow

The boot process follows the standard RISC-V boot protocol:

1. CPU starts executing at `0x8000_0000` in M-mode
2. OpenSBI firmware initializes:
    - Sets up M-mode trap vectors
    - Configures trap delegation to S-mode (`medeleg`, `mideleg`)
    - Initializes CLINT timer
    - Discovers available ISA extensions from `misa`
3. OpenSBI jumps to the kernel at `0x8020_0000`, passing the DTB pointer in `a1`
4. Linux kernel initializes:
    - Sets up S-mode page tables (SV39)
    - Initializes the PLIC and CLINT drivers
    - Mounts the VirtIO block device as root filesystem
    - Starts `/sbin/init` (BusyBox)
5. BusyBox init launches a login shell on the UART

## Performance Notes

- **O3 backend (width=8)**: Boots to shell in approximately 250M active cycles (1.60 active IPC)
- **In-order backend (width=1)**: Boots to shell in approximately 8-12 billion cycles
- **Host speed**: approximately 0.6-2.2 MHz simulated clock, so boot takes several minutes of wall-clock time
- **Memory hierarchy** matters: a realistic cache configuration (with MSHRs, L2/L3, DRAM timing) is essential for meaningful boot performance

### Out-of-Order (width=8)

```
host_seconds             416.9260 s
sim_cycles               247731327
sim_freq                 594.19 kHz
sim_insts                306010020
sim_ipc                  1.2352
sim_ipc_active           1.5960

CYCLE ACCOUNTING
  cycles.retiring        79044845 (31.91%)
  cycles.rob_empty       16550957 (6.68%)
  cycles.rob_stall       96139700 (38.81%)
  cycles.wfi             55995825 (22.60%)
  retire.per_cycle       0:68.1%  1:7.7%  2:6.7%  3+:17.5%
  retire.active          0:58.8%  1:10.0%  2:8.6%  3+:22.6%

PRIVILEGE BREAKDOWN
  cycles.user            25824642 (10.42%)
  cycles.kernel          218267482 (88.11%)
  cycles.machine         3639203 (1.47%)

PIPELINE STALLS
  stalls.memory          240 (0.00%)
  stalls.control         18995271 (7.67%)
  stalls.data            22095424 (8.92%)
  stalls.fu_structural   5901573 (2.38%)
  stalls.backpressure    484787 (0.20%)
  stalls.dispatch        15732923 (6.35%)
  stalls.checkpoint      9576 (0.00%)
  stalls.squash          30287660 (12.23%)
  stalls.rename_rebuild  2545723 (1.03%)

BRANCH PREDICTION (COMMITTED)
  bp.committed_lookups   35704632
  bp.committed_mispreds  2027449
  bp.committed_accuracy  94.32%

BRANCH PREDICTION (SPECULATIVE)
  bp.spec_lookups        62156792
  bp.spec_mispredicts    17725874
  bp.spec_accuracy       71.48%

MEMORY HIERARCHY
  L1-I   accesses: 141755255  | hits: 139309237  | miss_rate: 1.73%
  L1-D   accesses: 141389383  | hits: 136015198  | miss_rate: 3.80%
  L2     accesses: 7820203    | hits: 7525703    | miss_rate: 3.77%
  L3     accesses: 294500     | hits: 258216     | miss_rate: 12.32%
```

### In-Order (width=1)

```
host_seconds             392.5803 s
sim_cycles               882644951
sim_freq                 2248.32 kHz
sim_insts                382917708
sim_ipc                  0.4338
sim_ipc_active           0.4714

CYCLE ACCOUNTING
  cycles.retiring        382917708 (43.38%)
  cycles.rob_empty       7841598 (0.89%)
  cycles.rob_stall       421543310 (47.76%)
  cycles.wfi             70342335 (7.97%)
  retire.per_cycle       0:56.6%  1:43.4%  2:0.0%  3+:0.0%

PRIVILEGE BREAKDOWN
  cycles.user            92061892 (10.43%)
  cycles.kernel          766987718 (86.90%)
  cycles.machine         23595341 (2.67%)

PIPELINE STALLS
  stalls.memory          147797326 (16.74%)
  stalls.control         17754762 (2.01%)
  stalls.data            232645156 (26.36%)
  stalls.dispatch        51297903 (5.81%)

BRANCH PREDICTION (COMMITTED)
  bp.committed_accuracy  95.00%

BRANCH PREDICTION (SPECULATIVE)
  bp.spec_accuracy       75.36%

MEMORY HIERARCHY
  L1-I   accesses: 721721710  | hits: 719234554  | miss_rate: 0.34%
  L1-D   accesses: 156398623  | hits: 153852825  | miss_rate: 1.63%
  L2     accesses: 5032954    | hits: 4830768    | miss_rate: 4.02%
  L3     accesses: 202186     | hits: 168405     | miss_rate: 16.71%
```

The O3 backend achieves **1.60 active IPC** with the 8-wide pipeline, a **3.4x improvement** over the in-order backend (0.47 IPC). The in-order pipeline simulates faster on the host (2.2 MHz vs 0.6 MHz) due to simpler per-cycle logic. The in-order backend is dominated by data stalls (26%) and memory stalls (17%), while the O3 backend hides much of this latency in the reorder buffer.
