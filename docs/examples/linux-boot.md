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
