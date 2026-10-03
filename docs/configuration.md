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
| `width` | `int` | `4` | Instructions per cycle for every stage that has no width of its own |
| `fetch_width`, `decode_width`, `rename_width`, `issue_width`, `commit_width` | `int` | `width` | Per-stage widths, as gem5's `fetchWidth` … `commitWidth` |
| `writeback_width` | `int` | `width` | Results the out-of-order backend writes back per cycle, as gem5's `wbWidth`; the rest wait for later cycles |
| `vector_mem_width` | `int` | `vlen / 8`, at most `64` | Bytes one unit-stride vector memory access moves: the vector load-store datapath width, a power of two from 8 to 64 |
| `trap_latency` | `int` | `13` | Cycles from commit detecting a trap or interrupt to the squash into its handler; an interrupt first lets everything already fetched retire |
| `redirect_latency` | `int` | `2` (O3), `1` (in-order) | Cycles from execute resolving a misprediction, CSR write, fault or ordering violation to the squash into the redirect, as gem5's `iewToCommitDelay` + `commitToFetchDelay` and Minor's execute-to-fetch branch latch; commit retires nothing the pending squash will remove |
| `store_forward_latency` | `int` | L1D hit latency | Cycles from a load matching a store in the store buffer to its data reaching writeback, where a load the L1D answers takes the L1D hit latency; `1` is gem5's O3 LSQ, whose forwarded load writes back the cycle after it executes; `0` writes the load back in the cycle it matches (a forwarded vector span takes at least one cycle) |
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
    reset_interval=2000,
    history_lengths=[5, 15, 44, 130],
    tag_widths=[9, 9, 10, 10],
)
BranchPredictor.ScLTage(          # SC-L-TAGE + ITTAGE (highest accuracy)
    # TAGE parameters (defaults: the 64KB TAGE-SC-L's 36 banked tables)
    num_banks=36,
    table_size=1024,
    reset_interval=1024,  # CBP-5: allocation penalties before useful bits halve
    history="pc_bits",
    hashing="tage_sc_l",
    banking=BranchPredictor.TageBanking(
        short_factor=10, long_factor=20, first_long_bank=12,
        enabled=[...],  # one flag per bank; the default is gem5's noSkip
    ),
    # Loop predictor (2^log_size entries, 2^log_assoc ways)
    loop_log_size=5,
    loop_log_assoc=2,
    # Statistical corrector (defaults: Seznec's 64KB TAGE-SC-L)
    sc_counter_bits=6,
    sc_backward=BranchPredictor.ScGehl([40, 24, 10], log_entries=10, weight_init=7),
    sc_path=BranchPredictor.ScGehl([25, 16, 9], log_entries=9, weight_init=7),
    sc_local=[
        BranchPredictor.ScLocalGehl(256, index_shift=2, lengths=[11, 6, 3], log_entries=10),
        BranchPredictor.ScLocalGehl(16, index_shift=5, lengths=[16, 11, 6], log_entries=9, mix_pc=True),
        BranchPredictor.ScLocalGehl(16, index_shift=10, lengths=[9, 4], log_entries=10),
    ],
    sc_imli=BranchPredictor.ScGehl([8], log_entries=8, weight_init=7),
    sc_imli_history=BranchPredictor.ScGehl([10, 4], log_entries=9, weight_init=0),
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
MemDepPredictor.Blind()           # Conservative: loads wait for all older stores
MemDepPredictor.StoreSet(         # Store-set predictor (Chrysos & Emer 1998), default
    ssit_size=1024,               # Store Set ID Table entries
    lfst_size=1024,               # Last Fetched Store Table entries
)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `mem_dep_predictor` | `MemDepPredictor.*` | `StoreSet()` | Memory dependence predictor type |
| `ssit_size` | `int` | `1024` | SSIT entries (StoreSet only) — maps PC → store set ID |
| `lfst_size` | `int` | `1024` | LFST entries (StoreSet only) — maps store set ID → last dispatched store |

---

## Caches

Each cache level is configured independently:

```python
Cache(
    size="32KB",          # Size: "4KB", "32KB", "1MB", etc.
    line="64B",           # Line size (default: 64B)
    ways=8,               # Associativity
    latency=1,            # Hit latency in cycles
    mshr_count=8,         # Outstanding line fetches (0 = simulator default, 8)
    write_buffers=8,      # Victims in flight to the next level (0 = default, 8)
    targets_per_mshr=20,  # Requests one MSHR can hold (0 = default, 20)
    response_latency=1,   # Cycles from a fill to answering its requests
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

!!! tip "MSHRs and writeback buffers"
    Every level fetches at most `mshr_count` lines at a time and keeps at
    most `write_buffers` evicted lines in flight to the next level; while
    either is exhausted, or one MSHR holds `targets_per_mshr` requests
    (gem5's `tgts_per_mshr`), the cache blocks and later requests queue.
    Passing `0` (the Python default) leaves the simulator default in place
    (8, 8 and 20, gem5's L1 value; gem5's L2 uses 12); a `mshr_count=1`
    cache is a blocking cache that serialises its misses.

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
MemoryController.Simple(      # Fixed latency (default), serialised on a
    bandwidth_gib_s=12.8,     # bandwidth: each request busies the controller
)                             # for its bytes' time
MemoryController.DRAM(        # Row-buffer aware timing
    t_cas=14,                 # Column access strobe latency
    t_ras=14,                 # Row access strobe latency
    t_pre=14,                 # Precharge latency
    row_miss_latency=120,     # Full row-miss penalty
)
MemoryController.DDR5(        # Command-level DDR5 (see architecture/memory.md)
    speed_bin="4800B",        # JEDEC bin: "4800B" or "5600B"
    channels=2,               # Channels × 2 sub-channels each
    ranks_per_channel=2,
    bank_groups_per_rank=8,
    banks_per_group=4,
    row_bits=16,              # 16 Gb x8 devices
    column_bits=6,            # 4 KiB rows per sub-channel (64 B lines)
    read_queue_entries=64,
    write_queue_entries=64,
    write_high_watermark=54,  # Start draining writes at this depth
    write_low_watermark=32,   # Return to reads at this depth
    min_writes_per_switch=16,
    frontend_latency_ns=10,   # Controller pipeline, gem5 defaults
    backend_latency_ns=10,
    scheduler="FrFcfs",       # or "Fcfs"
    refresh="AllBank",        # or "SameBank"
    address_mapping="RoRaBaChCo",  # or "RoRaBaCoCh", "RoCoRaBaCh"
    power_down_idle_ns=None,  # e.g. 200 to enable rank power-down
    ecc="None",               # "SecDed" / "ChipKill"
    patrol_scrub_ns=None,     # e.g. 100_000 to enable patrol scrubbing
    timing={"t_rcd": 40},     # Per-field overrides in DRAM command clocks
)
```

The DDR5 controller runs at the DRAM command clock (half the data rate);
`Config(cpu_clock_mhz=...)` sets the core clock it converts to and from.
Its statistics appear under `memctrl0.ch<C>.sc<S>.*` (see
`Stats.query("memctrl0.**")`).

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
| `sim_control_base` | `int` | `0x0010_2000` | Sim-control device base address (guest stats reset / dump / exit) |
| `kernel_offset` | `int` | `0x0020_0000` | Kernel load offset from ram_base |
| `bus_width` | `int` | `8` | Bus width in bytes |
| `bus_latency` | `int` | `4` | Bus transaction latency in cycles |
| `clint_divider` | `int` | `10` | Timer tick divider (mtime increments every N cycles) |
| `cpu_clock_mhz` | `int` | `2400` | Core clock, used to convert between simulator cycles and the DDR5 command clock |
| `rtc_epoch_seconds` | `int` | `1767225600` | Wall-clock time the RTC reports at cycle zero (2026-01-01), advanced by simulated time so runs are reproducible |
| `hart_count` | `int` | `1` | Harts in the system, one per core (see [Multi-core](#multi-core)) |

---

## Multi-core

`hart_count=N` builds `N` single-threaded cores, each with its own
pipeline, branch predictor and private L1/L2, sharing the LLC, memory and
devices. Every hart has its own CLINT timer and software-interrupt slots
and its own PLIC contexts, and the generated device tree enumerates them.
Bare-metal programs start every hart at the entry point with `a0` holding
the hart id and `a1` the hart count.

```python
from rvsim import Config, Coherence, HomeAgent, Interconnect

config = Config(
    width=4,
    hart_count=4,
    coherence=Coherence(
        home_agent=HomeAgent.SnoopFilter(capacity_factor=1.5, ways=8),
        interconnect=Interconnect.Mesh(hop_latency=2, bytes_per_cycle=32),
    ),
)
```

With more than one core the private L2s become requesting agents on a
coherence fabric: MESI states in every private cache, a home agent at the
LLC that serialises requests per line and decides who is snooped, and an
interconnect that carries request, snoop, response and data messages on
separate virtual channels. The L2 is made inclusive of its L1s so snoops
are answered from its tags; `Cache.Exclusive()` is therefore rejected
with `hart_count > 1`. A single core builds no fabric and is unaffected.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `coherence.home_agent` | `HomeAgent.*` | `HomeAgent.SnoopFilter()` | Who must be snooped for a request |
| `coherence.interconnect` | `Interconnect.*` | `Interconnect.Crossbar()` | Message transport between the L2s and the home |
| `coherence.txn_entries` | `int` | `32` | Transactions the home can have live at once |

### Home agents

```python
HomeAgent.SnoopFilter(capacity_factor=1.5, ways=8)  # exact sharers + owner per tracked line (default)
HomeAgent.Broadcast()                                # track nothing; snoop every other core
```

The snoop filter tracks `capacity_factor` times the aggregate private L2
lines in a `ways`-way set-associative array. When a set is full, its least
recently used line is recalled (every holder invalidated) before a new
line is tracked, as Arm's snoop filter and AMD's probe filter do.

### Interconnects

```python
Interconnect.Crossbar(hop_latency=2, bytes_per_cycle=32)   # any port to any port (default)
Interconnect.Ring(hop_latency=2, bytes_per_cycle=32)       # bidirectional ring, shorter direction
Interconnect.Mesh(hop_latency=2, bytes_per_cycle=32)       # square 2-D mesh, XY routing
Interconnect.Torus(hop_latency=2, bytes_per_cycle=32)      # mesh with wraparound
Interconnect.Hypercube(hop_latency=2, bytes_per_cycle=32)  # dimension-order routing
```

`hop_latency` is the cycles a message spends per hop and `bytes_per_cycle`
the width of a port or link; a 64-byte data message on a 32-byte link
occupies it for two cycles. The crossbar is one hop; the routed networks
place the cores and the home on their nodes and charge every hop.

The fabric reports under `coherence.ha.*` (requests by kind, snoops,
cache-to-cache transfers, recalls, transaction latency) and
`coherence.interconnect.*` (messages, bytes, busy and blocked cycles);
each private L2 counts its snoops under `core<N>.l2.coherence.*`.

---

## General

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `trace` | `bool` | `False` | Enable per-instruction commit logging |
| `initial_sp` | `int` or `None` | `None` | Initial stack pointer (auto-configured if None) |
| `uart_quiet` | `bool` | `False` | Suppress UART output (useful for sweeps); shorthand for `console="quiet"` |
| `console` | `str` | `None` | Where the UART console connects: `"stdout"`, `"stderr"`, `"quiet"`, or `"captured"` (output kept in memory for `read_console()`, input given with `write_console()`); overrides the `uart_*` shorthands |
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
