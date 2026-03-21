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
    MemoryController, Prefetcher,
)

config = Config(
    width=4,
    backend=Backend.OutOfOrder(rob_size=128),
    branch_predictor=BranchPredictor.TAGE(),
    ram_size="256MB",
    l1i=Cache("32KB", ways=4, latency=1,
              prefetcher=Prefetcher.NextLine(degree=1)),
    l1d=Cache("32KB", ways=4, latency=1, mshr_count=16,
              prefetcher=Prefetcher.Stride(degree=1, table_size=64)),
    l2=Cache("256KB", ways=8, latency=8, mshr_count=32),
    l3=Cache("2MB", ways=16, latency=24),
    memory_controller=MemoryController.DRAM(
        t_cas=14, row_miss_latency=120,
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

- **O3 backend (width=4)**: Boots to shell in approximately 3-5 billion cycles
- **In-order backend (width=1)**: Boots to shell in approximately 8-12 billion cycles
- **Host speed**: approximately 0.8 MHz simulated clock, so boot takes several minutes of wall-clock time
- **Memory hierarchy** matters: a realistic cache configuration (with MSHRs, L2/L3, DRAM timing) is essential for meaningful boot performance

### Out-of-Order (width=4)

```
host_seconds             693.2677 s
sim_cycles               317664786
sim_freq                 458.21 kHz
sim_insts                314246742
sim_ipc                  0.9892
sim_ipc_active           1.1412

CYCLE ACCOUNTING
  cycles.retiring        104812792 (32.99%)
  cycles.rob_empty       15061527 (4.74%)
  cycles.rob_stall       155496730 (48.95%)
  cycles.wfi             42293737 (13.31%)
  retire.per_cycle       0:67.0%  1:9.7%  2:8.9%  3+:14.4%

PRIVILEGE BREAKDOWN
  cycles.user            35480805 (11.17%)
  cycles.kernel          275772748 (86.81%)
  cycles.machine         6411233 (2.02%)

PIPELINE STALLS
  stalls.control         15484588 (4.87%)
  stalls.data            57372999 (18.06%)
  stalls.fu_structural   1520218 (0.48%)
  stalls.dispatch        9285038 (2.92%)

BRANCH PREDICTION (COMMITTED)
  bp.committed_accuracy  96.45%

BRANCH PREDICTION (SPECULATIVE)
  bp.spec_accuracy       75.97%

MEMORY HIERARCHY
  L1-I   accesses: 95045393   | hits: 93163965   | miss_rate: 1.98%
  L1-D   accesses: 134649190  | hits: 129921239  | miss_rate: 3.51%
  L2     accesses: 6609379    | hits: 6366556    | miss_rate: 3.67%
  L3     accesses: 242823     | hits: 210340     | miss_rate: 13.38%
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

The O3 backend completes boot in **2.8x fewer simulated cycles** (318M vs 883M) with **2.3x higher IPC** (0.99 vs 0.43). The in-order pipeline simulates faster on the host (2.2 MHz vs 0.5 MHz) due to simpler per-cycle logic, so it finishes in less wall-clock time (393s vs 693s) despite taking far more simulated cycles. The in-order backend is dominated by data stalls (26%) and memory stalls (17%), while the O3 backend hides much of this latency in the reorder buffer.
