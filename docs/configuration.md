# Configuration

Every aspect of the simulated machine is runtime-configurable through the `Config` class. Parameters are flat (no nested objects) and use builder-style type classes for caches, predictors, and backends.

## Basic Usage

```python
from rvsim import Config, Cache, Backend, BranchPredictor, MemDepPredictor

config = Config(
    width=4,
    backend=Backend.OutOfOrder(rob_size=128),
    branch_predictor=BranchPredictor.TAGE(),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8),
    l2=Cache("256KB", ways=8, latency=10),
)
```

Use `replace()` to derive new configs from a base:

```python
base = Config(width=4, branch_predictor=BranchPredictor.TAGE())
narrow = base.replace(width=2)
wide = base.replace(width=8)
```

---

## Pipeline

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `width` | `int` | `4` | Fetch/decode/rename/retire width (instructions per cycle) |
| `backend` | `Backend.*` | `OutOfOrder()` | Pipeline backend: `Backend.InOrder()` or `Backend.OutOfOrder(...)` |
| `branch_predictor` | `BranchPredictor.*` | `TAGE()` | Branch predictor type |
| `btb_size` | `int` | `4096` | Branch target buffer entries |
| `btb_ways` | `int` | `4` | BTB associativity |
| `ras_size` | `int` | `32` | Return address stack depth |

### Backend: Out-of-Order

```python
Backend.OutOfOrder(
    rob_size=128,            # Reorder buffer entries
    issue_queue_size=32,     # Issue queue entries (CAM wakeup/select)
    store_buffer_size=32,    # Store buffer entries
    load_queue_size=32,      # Load queue entries (memory ordering)
    load_ports=2,            # Load ports per cycle
    store_ports=1,           # Store ports per cycle
    prf_gpr_size=256,        # Physical GPR file size
    prf_fpr_size=128,        # Physical FPR file size
    fu_config=Fu([...]),     # Functional unit pool (see below)
)
```

### Backend: In-Order

```python
Backend.InOrder()
```

No parameters — the in-order backend uses a fixed scoreboard-based pipeline. Pipeline width is controlled by the top-level `width` parameter.

### Functional Units (O3 only)

Configure the functional unit pool for the out-of-order backend:

```python
from rvsim import Fu

fu = Fu([
    Fu.IntAlu(count=4, latency=1),       # Integer ALU: add, sub, logic, shift
    Fu.IntMul(count=1, latency=3),       # Integer multiplier
    Fu.IntDiv(count=1, latency=35),      # Integer divider (non-pipelined)
    Fu.FpAdd(count=2, latency=4),        # FP add/sub/compare/convert
    Fu.FpMul(count=2, latency=5),        # FP multiply
    Fu.FpFma(count=2, latency=5),        # FP fused multiply-add
    Fu.FpDivSqrt(count=1, latency=21),   # FP divide/sqrt (non-pipelined)
    Fu.Branch(count=2, latency=1),       # Branch/jump resolution
    Fu.Mem(count=2, latency=1),          # Load/store address calculation
])
```

Omitting a FU type means the backend has zero units of that type. Make sure to include every type your workload exercises.

---

## Branch Predictor

```python
BranchPredictor.Static()          # Always predict not-taken
BranchPredictor.GShare()          # Global history XOR PC
BranchPredictor.Tournament(       # Two-level adaptive
    global_size_bits=12,
    local_hist_bits=10,
    local_pred_bits=10,
)
BranchPredictor.Perceptron(       # Neural predictor
    history_length=32,
    table_bits=10,
)
BranchPredictor.TAGE(             # Tagged geometric history length
    num_banks=4,
    table_size=2048,
    loop_table_size=256,
    reset_interval=2000,
    history_lengths=[5, 15, 44, 130],
    tag_widths=[9, 9, 10, 10],
)
BranchPredictor.ScLTage(          # SC-L-TAGE + ITTAGE (highest accuracy)
    # TAGE parameters
    num_banks=8,
    table_size=2048,
    loop_table_size=256,
    reset_interval=256_000,
    history_lengths=[5, 15, 44, 130, 380, 1024, 2048, 4096],
    tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
    # Statistical corrector
    sc_num_tables=6,
    sc_table_size=512,
    sc_counter_bits=3,
    # Indirect target TAGE
    ittage_num_banks=8,
    ittage_table_size=256,
    ittage_reset_interval=256_000,
)
```

---

## Memory Dependence Prediction

Controls how loads decide whether they can bypass unresolved older stores.

```python
MemDepPredictor.Blind()           # Conservative: loads wait for all older stores (default)
MemDepPredictor.StoreSet(         # Store-set predictor (Chrysos & Emer 1998)
    ssit_size=2048,               # Store Set ID Table entries
    lfst_size=256,                # Last Fetched Store Table entries
)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `mem_dep_predictor` | `MemDepPredictor.*` | `Blind()` | Memory dependence predictor type |
| `ssit_size` | `int` | `2048` | SSIT entries (StoreSet only) — maps PC → store set ID |
| `lfst_size` | `int` | `256` | LFST entries (StoreSet only) — maps store set ID → last dispatched store |

---

## Caches

Each cache level is configured independently:

```python
Cache(
    size="32KB",          # Size: "4KB", "32KB", "1MB", etc.
    line="64B",           # Line size (default: 64B)
    ways=8,               # Associativity
    latency=1,            # Hit latency in cycles
    mshr_count=8,         # MSHRs for non-blocking operation (0 = blocking)
    policy=ReplacementPolicy.LRU(),       # Eviction policy
    prefetcher=Prefetcher.Stride(),       # Hardware prefetcher
)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `l1i` | `Cache` | `32KB/4-way/1cy` | L1 instruction cache |
| `l1d` | `Cache` | `32KB/4-way/1cy` | L1 data cache |
| `l2` | `Cache` | `256KB/8-way/10cy` | L2 unified cache |
| `l3` | `Cache` or `None` | `None` | L3 cache (disabled by default) |
| `inclusion_policy` | `Cache.*` | `Cache.NINE()` | L1-L2 inclusion policy |
| `wcb_entries` | `int` | `0` | Write-combining buffer entries |

!!! tip "MSHRs matter"
    With `mshr_count=0` (the default), the L1D cache is **blocking** — every miss stalls the pipeline until the line arrives. Set `mshr_count=8` or higher for realistic non-blocking behavior where the O3 backend can execute other instructions while waiting for cache fills.

### Replacement Policies

```python
ReplacementPolicy.LRU()      # Least recently used (default)
ReplacementPolicy.PLRU()     # Pseudo-LRU (tree-based)
ReplacementPolicy.FIFO()     # First in, first out
ReplacementPolicy.Random()   # Random eviction
ReplacementPolicy.MRU()      # Most recently used
```

### Prefetchers

```python
Prefetcher.Off()                              # Disabled (default)
Prefetcher.NextLine(degree=1)                 # Prefetch next line on access
Prefetcher.Stride(degree=1, table_size=64)    # PC-indexed stride detection
Prefetcher.Stream(degree=1)                   # Sequential stream detection
Prefetcher.Tagged(degree=1)                   # Prefetch-on-prefetch
```

### Inclusion Policies

```python
Cache.NINE()        # No inclusion, non-exclusive (default)
Cache.Inclusive()    # L2 eviction back-invalidates matching L1 lines
Cache.Exclusive()   # L1 eviction swaps line into L2
```

---

## Memory

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `ram_size` | `str` or `int` | `"256MB"` | Main memory size |
| `memory_controller` | `MemoryController.*` | `Simple()` | Memory controller type |
| `tlb_size` | `int` | `32` | iTLB and dTLB entries (fully associative) |
| `l2_tlb_size` | `int` | `512` | Shared L2 TLB entries |
| `l2_tlb_ways` | `int` | `4` | L2 TLB associativity |
| `l2_tlb_latency` | `int` | `4` | L2 TLB hit latency in cycles |

### Memory Controller

```python
MemoryController.Simple()     # Fixed latency (default)
MemoryController.DRAM(        # Row-buffer aware timing
    t_cas=14,                 # Column access strobe latency
    t_ras=14,                 # Row access strobe latency
    t_pre=14,                 # Precharge latency
    row_miss_latency=120,     # Full row-miss penalty
)
```

---

## System

These parameters control the SoC memory map and device configuration. You normally don't need to change them.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `ram_base` | `int` | `0x8000_0000` | RAM base address |
| `uart_base` | `int` | `0x1000_0000` | UART base address |
| `disk_base` | `int` | `0x9000_0000` | VirtIO disk base address |
| `clint_base` | `int` | `0x0200_0000` | CLINT base address |
| `syscon_base` | `int` | `0x0010_0000` | SYSCON base address |
| `kernel_offset` | `int` | `0x0020_0000` | Kernel load offset from ram_base |
| `bus_width` | `int` | `8` | Bus width in bytes |
| `bus_latency` | `int` | `4` | Bus transaction latency in cycles |
| `clint_divider` | `int` | `10` | Timer tick divider (mtime increments every N cycles) |

---

## General

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `trace` | `bool` | `False` | Enable per-instruction commit logging |
| `initial_sp` | `int` or `None` | `None` | Initial stack pointer (auto-configured if None) |
| `uart_quiet` | `bool` | `False` | Suppress UART output (useful for sweeps) |
| `uart_to_stderr` | `bool` | `False` | Route UART output to stderr instead of stdout |

---

## Example Configurations

### Minimal embedded core

```python
Config(
    width=1,
    backend=Backend.InOrder(),
    branch_predictor=BranchPredictor.Static(),
    l1d=Cache("4KB", ways=1, latency=1),
    l1i=Cache("4KB", ways=1, latency=1),
    l2=None,
)
```

### High-performance O3 core

```python
Config(
    width=4,
    backend=Backend.OutOfOrder(
        rob_size=128,
        issue_queue_size=48,
        load_queue_size=32,
        store_buffer_size=32,
        prf_gpr_size=256,
        prf_fpr_size=128,
        fu_config=Fu([
            Fu.IntAlu(count=4, latency=1),
            Fu.IntMul(count=1, latency=3),
            Fu.IntDiv(count=1, latency=35),
            Fu.FpAdd(count=2, latency=4),
            Fu.FpMul(count=2, latency=5),
            Fu.FpFma(count=2, latency=5),
            Fu.FpDivSqrt(count=1, latency=21),
            Fu.Branch(count=2, latency=1),
            Fu.Mem(count=2, latency=1),
        ]),
    ),
    branch_predictor=BranchPredictor.ScLTage(),
    mem_dep_predictor=MemDepPredictor.StoreSet(),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8,
              prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
    l1i=Cache("32KB", ways=8, latency=1,
              prefetcher=Prefetcher.NextLine(degree=2)),
    l2=Cache("256KB", ways=8, latency=10, mshr_count=16),
    l3=Cache("4MB", ways=16, latency=30, mshr_count=32),
    memory_controller=MemoryController.DRAM(t_cas=14, row_miss_latency=120),
)
```

### Linux-capable system

See [Linux Boot](examples/linux-boot.md) for a complete config that boots Linux.
