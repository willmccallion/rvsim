# Configuration

Every aspect of the simulated machine is set through the `Config` class.
Its parameters are flat keywords; caches, predictors, backends, memory
controllers and the coherence fabric are small builder classes passed to
them. A `Config` is serialised to the Rust core when a simulator is built,
where it is validated as a whole: an unknown key, or a combination no
machine can have (for example a BTB whose set count is not a power of two,
or an exclusive L1/L2 with more than one core), raises `ValueError` before
anything runs.

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

Use `replace()` to derive new configs from a base; every `Config` keeps
its own copies of its components, so changing one never changes another:

```python
base = Config(width=4, branch_predictor=BranchPredictor.TAGE())
narrow = base.replace(width=2)
wide = base.replace(width=8)
```

`rvsim.presets` holds complete machines: `basic()`, `fast()`, `p550()`,
`cortex_a72()`, `m1()`, and `linux()` for a system that boots the bundled
Linux image. See [Benchmark Configs](examples/benchmark-configs.md).

---

## Pipeline

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `width` | `int` | `4` | Instructions per cycle for every stage that has no width of its own |
| `fetch_width`, `decode_width`, `rename_width`, `issue_width`, `commit_width` | `int` | `width` | Per-stage widths |
| `writeback_width` | `int` | `width` | Results the out-of-order backend writes back per cycle; the rest wait for later cycles |
| `trap_latency` | `int` | `13` | Cycles from commit detecting a trap or interrupt to the squash into its handler; an interrupt first lets everything already fetched retire |
| `redirect_latency` | `int` | `2` (O3), `1` (in-order) | Cycles from execute resolving a misprediction, CSR write, fault or ordering violation to the squash into the redirect; commit retires nothing the pending squash will remove |
| `store_forward_latency` | `int` | L1D hit latency | Cycles from a load matching a store in the store buffer to its data reaching writeback, where a load the L1D answers takes the L1D hit latency; `0` writes the load back in the cycle it matches (a forwarded vector span takes at least one cycle) |
| `backend` | `Backend.*` | `OutOfOrder()` | `Backend.OutOfOrder(...)` or `Backend.InOrder()` |
| `branch_predictor` | `BranchPredictor.*` | `TAGE()` | Direction predictor (see below) |
| `btb_size` | `int` | `4096` | Branch target buffer entries |
| `btb_ways` | `int` | `4` | BTB associativity; `btb_size / btb_ways` must be a power of two |
| `ras_size` | `int` | `32` | Return address stack depth |
| `mem_dep_predictor` | `MemDepPredictor.*` | `StoreSet()` | Memory dependence predictor (see below) |

### Backend: Out-of-Order

```python
Backend.OutOfOrder(
    rob_size=128,                 # Reorder buffer entries
    issue_queue_size=32,          # Unified issue queue entries (CAM wakeup/select)
    store_buffer_size=32,         # Store buffer entries
    load_queue_size=32,           # Load queue entries
    load_ports=2,                 # Loads issued per cycle
    store_ports=1,                # Stores issued per cycle
    prf_gpr_size=256,             # Physical integer registers
    prf_fpr_size=128,             # Physical floating-point registers
    fu_config=Fu([...]),          # Functional unit pool (see below); Fu() by default
    checkpoint_count=0,           # Rename-map checkpoints for branch recovery (0: rebuild from the ROB)
    squash_width=8,               # ROB entries commit squashes per cycle after a squash
    prf_vpr_size=64,              # Physical vector registers
    vec_chaining=True,            # Let a dependent vector op start on the first element group
    vec_store_buffer_size=8,      # In-flight vector stores
    vec_store_forwarding="byte_mask",  # Vector store-to-load forwarding: "byte_mask", "stall" or "off"
)
```

- **Register files.** `prf_gpr_size` and `prf_fpr_size` must each hold
  the 32 architectural registers plus one for every instruction that can
  be in flight with a destination; 32 + `rob_size` always suffices.
- **Squash recovery.** After a misprediction, trap or ordering violation,
  commit squashes the flushed ROB entries `squash_width` per cycle and
  rename waits until it has finished, one cycle more; the rename map
  itself is restored at once, from a checkpoint when the squashing branch
  has one.
- **Stores** issue in two halves when their data is not ready: the
  address as soon as the base register is, the data when the value is
  (see [Pipeline](architecture/pipeline.md)).
- **Vector store forwarding.** `byte_mask` forwards any bytes a vector
  store holds, as most out-of-order cores do; `stall` makes an overlapping
  load wait for the store to be written, as Saturn does; `off` treats every
  overlap as a stall.

### Backend: In-Order

```python
Backend.InOrder()
```

The in-order backend takes no parameters of its own. It issues in program
order, up to `issue_width` per cycle, with a scoreboard tracking operands;
its functional units are the default `Fu()` pool, and it has a 64-entry
ROB and a 16-entry store buffer.

### Functional Units

```python
from rvsim import Fu

fu = Fu([
    Fu.IntAlu(count=4, latency=1),       # add, sub, logic, shift, compare
    Fu.IntMul(count=1, latency=3),       # multiply (pipelined)
    Fu.IntDiv(count=1, latency=35),      # divide and remainder (not pipelined)
    Fu.FpAdd(count=2, latency=4),        # FP add, subtract, compare, convert
    Fu.FpMul(count=2, latency=5),        # FP multiply
    Fu.FpFma(count=2, latency=5),        # FP fused multiply-add
    Fu.FpDivSqrt(count=1, latency=21),   # FP divide and square root (not pipelined)
    Fu.Branch(count=2, latency=1),       # branch and jump resolution
    Fu.Mem(count=2, latency=1),          # load and store address generation
    Fu.VecIntAlu(count=1, latency=1),    # vector integer arithmetic and logic
    Fu.VecIntMul(count=1, latency=3),    # vector integer multiply
    Fu.VecIntDiv(count=1, latency=20),   # vector integer divide (not pipelined)
    Fu.VecFpAlu(count=1, latency=4),     # vector FP add, compare, convert
    Fu.VecFpFma(count=1, latency=5),     # vector FP multiply and fused multiply-add
    Fu.VecFpDivSqrt(count=1, latency=20),  # vector FP divide and square root (not pipelined)
    Fu.VecMem(count=1, latency=1),       # vector load and store address generation
    Fu.VecPermute(count=1, latency=1),   # slides, gathers, compress, moves
])
```

The list above is `Fu()`, the default. A scalar unit type left out of a
`Fu` list has no units, so an instruction that needs one never issues:
include every scalar type your workload uses. A vector unit type left out
gets one unit with the default latency. A vector instruction's time on its
unit also scales with `vl` over the number of lanes (`num_vec_lanes`,
below).

---

## Branch Prediction

```python
BranchPredictor.Static()          # Always predicts not-taken
BranchPredictor.GShare()          # PC XOR global history, 2-bit counters
BranchPredictor.Tournament(       # gem5's TournamentBP (Alpha 21264): local, global, choice
    global_size_bits=12,
    local_hist_bits=10,
    local_pred_bits=10,
)
BranchPredictor.Perceptron(       # Perceptron predictor
    history_length=32,
    table_bits=10,
)
BranchPredictor.TAGE(             # TAGEBase-style TAGE (defaults shown)
    num_banks=8,
    table_size=2048,
    reset_interval=256_000,
    history_lengths=[5, 11, 22, 44, 89, 178, 356, 712],
    tag_widths=[8, 8, 9, 9, 10, 10, 11, 11],
)
BranchPredictor.ScLTage()         # 64KB TAGE-SC-L with ITTAGE (Seznec's CBP-5 configuration)
```

`TAGE` and `ScLTage` take further keywords for every structure of the
predictor (allocation and update rules, history kind, hashing, banking,
the bimodal table, USE_ALT_ON_NA counters); `ScLTage` adds the loop
predictor (`loop_*`), the statistical corrector (`sc_*`, with
`BranchPredictor.ScGehl` and `BranchPredictor.ScLocalGehl` components) and
ITTAGE (`ittage_*`). Their defaults reproduce Seznec's 64KB TAGE-SC-L.
[Branch Prediction](architecture/branch-prediction.md) describes each
parameter.

---

## Memory Dependence Prediction

Controls how a load decides whether it may issue ahead of older stores
whose addresses are not known yet.

```python
MemDepPredictor.Blind()           # Loads wait for every older store's address
MemDepPredictor.StoreSet(         # Store-set predictor (Chrysos & Emer 1998), the default
    ssit_size=1024,               # Store Set ID Table entries (PC -> store set)
    lfst_size=1024,               # Last Fetched Store Table entries (store set -> last store)
)
```

The store-set predictor learns from ordering violations and wipes both
tables every 250,000 memory instructions.

---

## Caches

Each level is configured independently. The builder's own defaults (a
4 KiB direct-mapped cache) are shown; `Config`'s default levels are in the
table below.

```python
Cache(
    size="4KB",           # "4KB", "32KB", "1MB", or bytes
    line="64B",           # Line size; at least the 64-byte block CBOs act on
    ways=1,               # Associativity
    latency=1,            # Tag and data access latency in cycles
    response_latency=1,   # Cycles from a fill arriving to answering its requests
    mshr_count=0,         # Lines fetched at once (0 = the default, 8)
    write_buffers=0,      # Evicted lines in flight to the next level (0 = the default, 8)
    targets_per_mshr=0,   # Requests one MSHR can hold (0 = the default, 20)
    policy=None,          # Eviction policy; ReplacementPolicy.LRU() when None
    prefetcher=None,      # Hardware prefetcher; Prefetcher.Off() when None
)
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `l1i` | `Cache` or `None` | 32 KiB, 4-way, 1 cycle, next-line prefetch | L1 instruction cache |
| `l1d` | `Cache` or `None` | 32 KiB, 4-way, 1 cycle, stride prefetch | L1 data cache |
| `l2` | `Cache` or `None` | 256 KiB, 8-way, 10 cycles | Private L2 |
| `l3` | `Cache` or `None` | `None` | Shared last-level cache |
| `inclusion_policy` | `Cache.*` | `Cache.NINE()` | Relationship between L1 and L2 |
| `wcb_entries` | `int` | `0` | Write-combining buffer entries between the store buffer and the L1D (0 = none) |
| `load_prefetcher` | `LoadPrefetcher.Stride` or `None` | `None` | The load/store unit's load prefetcher (see below) |
| `store_prefetcher` | `StorePrefetcher.Stream` or `None` | `None` | The L1D's store-miss prefetcher, which fills the L2 (see below) |

A load that hits takes one cycle of address generation plus the L1D's
`latency` to reach its dependents, so `latency=3` models a 4-cycle
load-to-use. `None` disables a level.

!!! tip "MSHRs and writeback buffers"
    Every level fetches at most `mshr_count` lines at a time and keeps at
    most `write_buffers` evicted lines in flight to the next level; while
    either is exhausted, or one MSHR holds `targets_per_mshr` requests, the
    cache blocks and later requests queue. Passing `0` leaves the simulator
    default in place (8, 8 and 20); `mshr_count=1` gives a blocking cache
    that serialises its misses.

### Replacement Policies

```python
ReplacementPolicy.LRU()      # Least recently used (default)
ReplacementPolicy.PLRU()     # Tree pseudo-LRU
ReplacementPolicy.FIFO()     # First in, first out
ReplacementPolicy.Random()   # Random eviction
ReplacementPolicy.MRU()      # Most recently used
```

### Prefetchers

```python
Prefetcher.Off()                              # None (the default for Cache())
Prefetcher.NextLine(degree=1)                 # The next `degree` lines on every access
Prefetcher.Stride(degree=1, table_size=64)    # PC-indexed constant-stride detection
Prefetcher.Stream(degree=1)                   # Ascending or descending streams
Prefetcher.Tagged(degree=1)                   # Next lines on a miss or a first use of a prefetched line
```

A prefetch is a real fetch: it takes an MSHR (never the last free one)
and travels down the hierarchy like a demand miss. A cache sees physical
addresses only, so its prefetcher never crosses the 4 KiB page of the
access that triggered it.

### Load and store prefetchers

The L1D's prefetching on a real core lives in the load/store unit, where
each load's PC, virtual address and translation are known. These follow
the Cortex-A72's documented prefetcher; the
[memory hierarchy page](architecture/memory.md#hardware-prefetching) gives
the design and its sources.

```python
LoadPrefetcher.Stride(
    table_size=64,        # PC-indexed entries, a power of two
    l1_lines=4,           # Lines kept ahead in the L1D
    l2_lines=0,           # Lines kept ahead in the L2 alone (the A72 keeps 22)
    page_boundary=None,   # PageBoundary.Stop() (when None) or PageBoundary.CrossWithTlb()
)
StorePrefetcher.Stream(
    streams=4,            # Runs of store misses tracked at once
    l2_lines=8,           # Lines kept ahead in the L2, with write permission
)
```

`PageBoundary.Stop()` keeps a stream inside the page of the load that
trained it, at that page's size; `PageBoundary.CrossWithTlb()` continues
into the next page when the data TLB holds its translation and drops the
prefetch when it does not. Both are passed to `Config`:

```python
Config(
    load_prefetcher=LoadPrefetcher.Stride(l1_lines=1, l2_lines=22,
                                          page_boundary=PageBoundary.CrossWithTlb()),
    store_prefetcher=StorePrefetcher.Stream(),
)
```

### Inclusion Policies

```python
Cache.NINE()        # Neither inclusive nor exclusive (default)
Cache.Inclusive()   # An L2 eviction back-invalidates the L1 copies
Cache.Exclusive()   # L1 victims go to the L2; an L1 fill takes the L2's copy
```

---

## Memory and Translation

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `ram_size` | `str` or `int` | `"256MB"` | Main memory size |
| `memory_controller` | `MemoryController.*` | `Simple()` | Memory controller (see below) |
| `tlb_size` | `int` | `64` | Entries in each of the instruction and data L1 TLBs |
| `tlb_ways` | `int` | `0` | L1 TLB associativity; `0` is fully associative |
| `l2_tlb_size` | `int` | `0` | Entries in the shared L2 TLB; `0` disables it |
| `l2_tlb_ways` | `int` | `4` | L2 TLB associativity |
| `l2_tlb_latency` | `int` | `4` | L2 TLB hit latency in cycles |
| `paging_mode_max` | `str` | `"sv57"` | Strongest paging mode `satp` accepts (`"bare"`, `"sv39"`, `"sv48"`, `"sv57"`); a stronger mode written to `satp` reads back as Bare, which makes a kernel fall back |
| `misaligned_access_trap` | `bool` | `False` | Raise address-misaligned exceptions instead of performing misaligned accesses in hardware |
| `svadu` | `bool` | `False` | Implement Svadu: with `menvcfg.ADUE` set the page-table walker sets A and D bits itself; otherwise a missing A or D bit faults (Svade) |

### Memory Controllers

```python
MemoryController.Simple(      # Fixed latency (default), serialised on a bandwidth:
    latency=120,              # Core cycles from the controller starting a request to its data
    bandwidth_gib_s=12.8,     # each request busies the controller for its bytes' time
)
MemoryController.DRAM(        # Row-buffer DRAM: per-bank open rows and refresh
    t_cas=14,                 # Column access, cycles
    t_ras=14,                 # Row activate, cycles
    t_pre=14,                 # Precharge, cycles
)
MemoryController.DDR5(        # Command-level JEDEC DDR5 (see Memory Hierarchy)
    speed_bin="4800B",        # "4800B" or "5600B"
    channels=2,               # Channels; each has two 32-bit sub-channels
    subchannels_per_channel=2,
    ranks_per_channel=2,
    bank_groups_per_rank=8,
    banks_per_group=4,
    row_bits=16,
    column_bits=6,            # Rows of 64 << column_bits bytes
    read_queue_entries=64,
    write_queue_entries=64,
    write_high_watermark=54,  # Start draining writes at this depth
    write_low_watermark=32,   # Return to reads at this depth
    min_writes_per_switch=16,
    frontend_latency_ns=10,   # Controller pipeline
    backend_latency_ns=10,
    scheduler="FrFcfs",       # or "Fcfs"
    refresh="AllBank",        # or "SameBank"
    address_mapping="RoRaBaChCo",  # or "RoRaBaCoCh", "RoCoRaBaCh"
    power_down_idle_ns=None,  # e.g. 200 to enable rank power-down
    ecc="None",               # "SecDed" or "ChipKill"
    patrol_scrub_ns=None,     # e.g. 100_000 to enable patrol scrubbing
    timing=None,              # Per-field overrides in DRAM command clocks, e.g. {"t_rcd": 40}
)
```

On the DRAM controller a row hit costs `t_cas` and a row miss
`t_pre + t_ras + t_cas`. The DDR5 controller runs at the DRAM command clock
(half the data rate) and converts to and from the core clock through `cpu_clock_mhz`; its statistics appear under
`memctrl0.ch<C>.sc<S>.*`. Every controller sits behind the system bus, so a
miss to memory also pays `bus_latency` each way.

---

## Vector Extension

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `vlen` | `int` | `128` | Vector register length in bits, a power of two from 128 to 2048 |
| `num_vec_lanes` | `int` | `vlen / 64`, at least 1 | 64-bit lanes the vector units process per cycle |
| `vector_mem_width` | `int` | `vlen / 8`, at most `64` | Bytes one unit-stride vector memory access moves (the vector load-store datapath), a power of two from 8 to 64 |

ELEN is 64 and Zvfh is implemented.

---

## System

These parameters set the SoC's memory map, clocks and devices. You
normally need to change only `cpu_clock_mhz`, `hart_count` and the
console.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `cpu_clock_mhz` | `int` | `2400` | Core clock: converts between cycles and nanoseconds for the DDR5 controller, device latencies and the RTC |
| `hart_count` | `int` | `1` | Harts in the system, one per core (see [Multi-core](#multi-core)) |
| `bus_width` | `int` | `8` | System bus width in bytes |
| `bus_latency` | `int` | `4` | System bus latency in cycles, each way |
| `device_latency_ns` | `int` | `100` | Time every device takes to answer a register access |
| `device_latency_ns_overrides` | `dict` | `None` | Per-device access latency by name (`UART0`, `CLINT`, `PLIC`, `VirtIO-Blk`, `SysCon`, `GoldfishRTC`, `HTIF`) |
| `clint_divider` | `int` | `10` | CPU cycles per `mtime` tick |
| `rtc_epoch_seconds` | `int` | `1767225600` | Wall-clock time the RTC reports at cycle zero (2026-01-01), advanced by simulated time so runs are reproducible |
| `ram_base` | `int` | `0x8000_0000` | RAM base address |
| `uart_base` | `int` | `0x1000_0000` | UART base address |
| `disk_base` | `int` | `0x9000_0000` | VirtIO disk base address |
| `clint_base` | `int` | `0x0200_0000` | CLINT base address |
| `syscon_base` | `int` | `0x0010_0000` | SYSCON base address |
| `sim_control_base` | `int` | `0x0010_2000` | Sim-control device base address (guest statistics reset, dump and exit) |
| `kernel_offset` | `int` | `0x0020_0000` | Kernel load offset from `ram_base` |

---

## Multi-core

`hart_count=N` builds `N` single-threaded cores, each with its own
pipeline, branch predictor, TLBs and private L1 and L2, sharing the LLC,
memory and devices. Every hart has its own CLINT timer and
software-interrupt registers and its own PLIC contexts, and the generated
device tree enumerates them. Bare-metal programs start every hart at the
entry point with `a0` holding the hart id and `a1` the hart count.

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
with `hart_count > 1`. A single core builds no fabric.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `coherence.home_agent` | `HomeAgent.*` | `HomeAgent.SnoopFilter()` | Who must be snooped for a request |
| `coherence.interconnect` | `Interconnect.*` | `Interconnect.Crossbar()` | Message transport between the L2s and the home |
| `coherence.txn_entries` | `int` | `32` | Transactions the home can have live at once |

### Home agents

```python
HomeAgent.SnoopFilter(capacity_factor=1.5, ways=8)  # exact sharers and owner per tracked line (default)
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
each private cache counts its snoops under
`core<N>.cache.<level>.coherence.*`. See [Multi-core](architecture/multicore.md).

---

## General

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `trace` | `bool` | `False` | Emit a trace event at every pipeline stage an instruction passes. `RUST_LOG` selects which are printed: `rvsim=trace` for all, or targets such as `rvsim::commit=trace`, `rvsim::mem=trace` and `rvsim::fwd=trace` |
| `initial_sp` | `int` or `None` | `None` | Stack pointer a bare-metal program starts with; `ram_base + 16 MiB` when unset |
| `console` | `str` or `None` | `None` | Where the UART connects: `"stdout"`, `"stderr"`, `"quiet"`, or `"captured"` (kept in memory for `read_console()`, input given with `write_console()`); overrides the two shorthands below |
| `uart_quiet` | `bool` | `False` | Shorthand for `console="quiet"` |
| `uart_to_stderr` | `bool` | `False` | Shorthand for `console="stderr"` |

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

### High-performance out-of-order core

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
        checkpoint_count=32,
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
    l1d=Cache("32KB", ways=8, latency=3, mshr_count=8,
              prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
    l1i=Cache("32KB", ways=8, latency=1,
              prefetcher=Prefetcher.NextLine(degree=2)),
    l2=Cache("256KB", ways=8, latency=12, mshr_count=16),
    l3=Cache("4MB", ways=16, latency=30, mshr_count=32),
    memory_controller=MemoryController.DDR5(speed_bin="5600B"),
    l2_tlb_size=1024,
)
```

### Linux-capable system

`presets.linux()` places a core in a system that boots the bundled Linux
image; see [Linux Boot](examples/linux-boot.md).
