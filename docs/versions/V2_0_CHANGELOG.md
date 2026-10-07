# Changelog — v2.0.0

Released: 2026-10-07

The largest release so far. rvsim now models multi-core systems with a
MESI coherence fabric, implements the RISC-V vector extension with its
crypto and bit-manipulation sub-extensions, and has a rebuilt
event-driven memory system: non-blocking caches at every level, a
command-level JEDEC DDR5 controller and memory accesses that take effect
where they are served. Statistics are one tree keyed by path, with
metadata, queries, derived metrics and region measurement, and every stat
is checked against hand-counted programs. Linux boots on eight coherent
cores, benchmarks run inside it with `rvsim bench`, and `Session` caches
the boot so a measurement starts at the shell. The pipelines were
reworked to follow real cores and gem5's O3 CPU, against which the
simulator now measures its own error. The Rust crate's model is
crate-private behind `Simulator`, and the Python API was reorganised
around it.

## Added

### ISA

- **Vector extension (V).** RVV 1.0 with ELEN 64 and a configurable VLEN,
  a power of two from 128 to 2048 bits (`Config(vlen=...)`, default 128).
  Unit-stride, strided, indexed, segment, fault-only-first, mask and
  whole-register loads and stores; integer, fixed-point (`vxrm`, `vxsat`)
  and floating-point arithmetic with widening and narrowing forms;
  reductions; mask operations; slides, gathers and compress; every LMUL
  including the fractional ones, with tail- and mask-agnostic policies.
  The vector CSRs `vstart`, `vxsat`, `vxrm`, `vcsr`, `vl`, `vtype` and
  `vlenb`, and `mstatus.VS` with its Off, Initial, Clean and Dirty states.
- **Vector sub-extensions.** Zvfh (half precision), Zvbb (vector bit
  manipulation), Zvbc (carry-less multiply), Zvkn (AES, SHA-256 and
  SHA-512 with Zvkb), Zvks (SM4 and SM3) and Zvkg (GHASH).
- **Scalar extensions.** Zba, Zbb, Zbc and Zbs bit manipulation, Zbkb and
  Zbkx for cryptography, Zfh half-precision floating point, Zicbom
  (`cbo.clean`, `cbo.flush`, `cbo.inval`) and Zicboz (`cbo.zero`) on a
  64-byte block, carried through every cache to memory.
- **Privileged architecture.** Sv48 and Sv57 paging alongside Sv39, capped
  by `Config(paging_mode_max=...)`; Svadu hardware A/D updates under
  `menvcfg.ADUE` (Svade remains the default); Sstc's `stimecmp`; Sdtrig
  debug triggers on execute, load and store addresses; `mcountinhibit`;
  `menvcfg` and `senvcfg` with the fields gating the cache-block
  operations; reserved PTE bits fault; `misa` reports V and drives the
  device tree's ISA string, and `misa_override` takes an ISA string.
- **Conformance.** The chipsalliance `riscv-vector-tests` suite is
  generated and checked against spike's signatures; riscv-tests and the
  vector tests run on every pipeline configuration (`make test-all`), and
  a smoke subset of both runs in CI on every pull request.

### Multi-core

- `Config(hart_count=N)` builds N cores, each with its own pipeline, TLBs,
  branch predictor and private L1 and L2 caches, behind a shared LLC.
- A MESI coherence fabric with CHI-like messages on request, response,
  snoop and data channels: a broadcast home agent or a snoop-filter home
  agent with recalls (`Coherence(home_agent=...)`), over a crossbar, ring,
  mesh, torus or hypercube interconnect whose links have a hop latency and
  a width (`Interconnect.*`). Dirty writebacks and snoop answers carry the
  line.
- Per-hart CLINT timers and software interrupts, per-hart PLIC contexts,
  and a device tree that lists every hart and advertises Sstc.
- LR/SC reservations and AMOs that hold across harts, device DMA that
  breaks reservations, and a write log so a load squashed by another
  hart's write replays.
- Every hart is reachable from Python (`Simulator.harts`, per-hart
  registers, CSRs and PCs), traces carry their hart, and an idle core's
  cycles are counted rather than ticked (`skip_idle_cores`).
- A coherence audit (`Simulator::audit_coherence` in Rust) and, with
  `Simulator.audit_caches = True`, a check of every cache and coherence
  invariant after every event.

### Memory system

- **Event-driven components.** Caches, the bus, memory controllers and
  devices exchange timed packets through an event queue. The bus is
  occupied for each transaction's transfer time; device register accesses
  follow gem5's bus and device timing; virtio DMA moves over the bus.
- **Accesses take effect where they are served.** The caches hold tags and
  the data lives in one memory image: a load reads and a store writes at
  the first cache holding the line with the permission it needs, AMOs and
  store-conditionals perform in the L1D at the ROB head, and a request no
  cache serves takes effect at the memory controller.
- **Non-blocking caches at every level** with MSHRs that coalesce requests
  up to a target limit, a writeback buffer, blocking when either is full,
  a configurable response latency, and NINE, inclusive (with
  back-invalidation) or exclusive inclusion between the L1s and the L2.
- **DDR5.** `MemoryController.DDR5()` schedules JEDEC commands per bank
  against JESD79-5B timing derived from a speed bin (`4800B`, `5600B`):
  channels and sub-channels, ranks, bank groups and banks with a
  configurable address mapping; bounded, posted read and write queues with
  write merging; FR-FCFS or FCFS scheduling; all-bank or same-bank
  refresh; rank power-down; and an ECC patrol scrubber. It runs in the
  DRAM clock domain. The simple controller serialises on a bandwidth.
- **TLBs** are set-associative with superpage entries, keep ASID-tagged
  entries across `satp` writes, and an optional L2 TLB charges its latency
  on a hit; the page-table walker reads PTEs through the L1D and sets A
  and D bits as the configured extension requires.
- **Prefetchers** follow published hardware: a load prefetcher in the
  load/store unit (`LoadPrefetcher.Stride`, PC-indexed, virtually
  addressed, filling the L1D and the L2, stopping at or crossing a page
  through the TLB), an L1D store-miss prefetcher filling the L2
  (`StorePrefetcher.Stream`), and cache-side next-line, stride, stream and
  tagged prefetchers that keep to the 4 KiB page. A prefetch is a real
  fetch that takes an MSHR.
- A load or store that crosses a cache line is split into two cache
  requests, and one that crosses a page translates both pages.

### Pipelines

- **Out-of-order backend.** Every stage has its own width; rename
  allocates from the previous cycle's free entries; serializing
  instructions hold rename until the ROB drains; system instructions and
  device reads execute from the ROB head; faults are taken at commit;
  stores issue their address ahead of their data; loads wake their
  dependents when their data returns; writeback is limited to
  `writeback_width`; a squash is taken `redirect_latency` cycles after the
  result and the ROB drains at `squash_width` a cycle, as in gem5's O3.
  `Config(store_forward_latency=...)` sets the store-to-load forwarding
  latency.
- **Fetch** forms one line-sized group at a time through a fetch buffer,
  predicts from the BTB alone and lets decode redirect on a BTB miss or a
  stale target; instructions straddling a line or a page fetch both
  halves.
- **In-order backend.** Superscalar issue onto the same functional-unit
  pool as the out-of-order backend, with real unit latencies, vector loads
  and stores through the memory stages, and the same serialization rules.
- **Vector execution** takes its unit for a time set by `vl` and
  `num_vec_lanes`, with optional chaining (`vec_chaining`); vector loads
  and stores move up to `vector_mem_width` bytes per L1D access, through a
  vector store buffer that forwards to younger loads.
- **Memory ordering.** The store-set predictor follows gem5's (and is the
  default); acquire and release atomics, fences and cache-block
  operations order loads and stores; store-buffer slots are held until
  the cache takes the write; a write-combining buffer merges committed
  stores.
- **Branch prediction.** TAGE-SC-L rebuilt after Seznec's 64KB CBP-5
  predictor and gem5's TAGEBase: banked tables indexed with path history,
  a loop predictor with speculative counts, a statistical corrector,
  several `USE_ALT_ON_NA` counters, a circular history with per-branch
  checkpoints, and ITTAGE trained on committed indirect jumps. The
  Tournament predictor is gem5's TournamentBP; GShare, Tournament and the
  perceptron train on the history they predicted with, and every
  predictor keeps a record per prediction to undo squashes from.

### Statistics

- **One tree keyed by path.** Every counter has a path that reads as a
  sentence (`core0.pipeline.stalls.control`, `hart1.traps`,
  `core0.cache.l2.prefetches.useful`, `memctrl0.ch0.sc1.row_hits`,
  `coherence.ha.snoops_sent`, `system.retired_insts`). Subjects are
  numbered (`core<N>`, `hart<N>`), never named after a backend or
  predictor class, so scripts survive a configuration change, and every
  cache level has the same counters.
- **Metadata at the source.** Each stat registers a description, a unit
  and a kind (accumulated, gauge or rate) when the simulator is built, so
  the whole tree, zeros included, exists before a run and the summary
  formats itself. Histograms (`Stats::register_histogram`) record count,
  sum, mean, minimum and maximum.
- **Derived metrics are stats.** IPC, CPI, branch accuracy, miss rates,
  prefetch accuracy and DDR5 row-hit rate and bus utilisation are stored
  as formulas and computed from their operands, so every consumer reads
  the same value.
- **Queries.** `stats.query("core*.cache.l1d.misses").sum()`,
  `stats.query("**.misses").by_subject()`, `stats.subjects()` and
  `stats.summary([...])`; a malformed pattern is an error, not an empty
  result.
- **Measuring a region.** `stats - earlier` subtracts every counter and
  recomputes the derived ones; `Simulator.reset_stats()`; guest software
  dumps labelled snapshots through the sim-control device, read with
  `stats_dumps()` and subtracted with `stats_between(start, end)`.
- **New counters**, including per-hart retired instructions, traps and
  cycles per privilege mode; pipeline stall cycles by cause (control,
  fetch wait, data, ordering, functional unit, backpressure, dispatch,
  checkpoint, serialize, squash); flushes by cause with the instructions
  they drop; retired instructions by class (scalar, atomic, FP, vector
  integer, FP, memory, crypto and misc); a retire-width histogram;
  functional-unit busy cycles per unit type; committed and speculative
  prediction accuracy and decode redirects; memory-dependence predictions
  and violations; load-queue replays, split stores and coherence replays;
  write-combining coalesces and drains; load prefetches sent and dropped
  by reason; per cache hits, misses, MSHR hits, blocked requests, fills,
  evictions, writebacks, back-invalidations, probes, maintenance
  operations, prefetches issued, late, useful, unused, page-crossing,
  dropped and store-stream, and coherence snoops, invalidations,
  downgrades, upgrades and upgrade retries; the home agent's requests by
  kind, snoops, cache-to-cache transfers, recalls, snoop-filter hits and
  misses and serialised requests; interconnect messages, bytes and busy
  and blocked cycles; and per DDR5 sub-channel and bank reads, writes,
  merges, activates, precharges, refreshes, row hits and misses,
  power-down entries, bus occupancy, admission stalls and latency and
  queue-depth histograms.
- **Every stat is checked.** An accounting suite runs small programs whose
  counts follow from their code and checks each stat against an exact
  count, a relation with other stats or a contrast; a gate fails when a
  registered stat has no check.
- Comparisons (`Stats.tabulate`, `Sweep.run().compare`), `rvsim --watch`,
  `--json` and the analysis examples read stats by path.

### Running workloads

- **Presets.** `rvsim.presets` has `basic()`, `fast()` (an Apple M4
  P-core class 8-wide core), `linux()` (the bundled image's system around
  any core), `cortex_a72()`, `m1()` and `p550()`. The A72 and P550 presets
  are calibrated against measured hardware latencies, and each value cites
  its source.
- **Linux.** `make run-linux` boots Linux 6.6 through OpenSBI on eight
  coherent cores over a mesh with four DDR5 channels;
  `tools/boot_linux.py` picks the harts, interconnect, memory and speed
  bin. `Simulator(config, kernel=..., disk=..., firmware=..., dtb=...)`
  boots from Python.
- **Sessions.** `Session` drives a workload in phases: `fast_forward` to a
  stop (cached as a checkpoint, so a later run starts at the login shell),
  `switch` to another core configuration, `warm_up`, `measure` a shell
  command or a run to a stop as a `Region`, and `send`, `expect` and
  `shell` to drive the console.
- **`rvsim bench`** runs CoreMark, Dhrystone, Whetstone, STREAM, mbw,
  lmbench's `lat_mem_rd` and stress-ng inside Linux on any preset or
  config file and reports cycles, IPC and misses per thousand
  instructions; the benchmarks and an `rvsim` guest tool are built into
  the root filesystem.
- **Sim control.** A device through which guest software resets and dumps
  stats, ends the run or stops the host's run at a labelled point, with
  helpers for bare-metal programs (`software/libc/rvsim.h`) and Linux
  (`rvsim run START END CMD`).
- **Run control.** `Simulator.run_to` stops at any of several PCs, a cycle
  or instruction count, console output or a guest's sim-control stop, and
  `Session` stops compose them (`Pc`, `Cycles`, `Console`, `Marker`,
  `Exit`, `AnyOf`);
  `skip_idle_cores` skips cycles in which only time passes.
- **Checkpoints** carry every hart's architectural state, device state and
  the disk's written sectors, skip pages of zeros, record their version,
  and drain the pipelines before saving and restoring.
- **Tracing.** Pipeline and trap events carry their hart and cycle and can
  be filtered by hart, cycle and cause; device, loader, HTIF and DMA
  messages are `tracing` events shown with `RUST_LOG`.

### Measuring against references

- `make compare-gem5` runs I/O-free programs on rvsim and gem5 across
  machine variants from one description; the documentation's "Error
  against gem5" page records the per-kernel cycle error and its known
  causes.
- `tools/diag/latency_probe.py` measures a configuration's load-to-use
  latency at each cache level, and `tools/diag/linux_bench.py` reports
  CoreMark/MHz and DMIPS/MHz for comparison with hardware.
- Cycle baselines recorded on every preset catch unintended timing
  changes.

### Documentation

- A design page and fourteen recorded design decisions; architecture
  pages for the pipelines, memory system, multi-core fabric, SoC devices,
  ISA and stats (with a catalogue of every stat path); every configuration
  parameter with its default; Linux boot and benchmark guides; and an API
  reference generated from the docstrings.

## Breaking changes

- **Python API.**
  - `Simulator` is built directly, `Simulator(config, binary=...)` or
    `Simulator(config, kernel=..., disk=...)`; the builder
    (`Simulator().config(...).binary(...).build()`) and the `Cpu` class it
    returned are gone, as are the gem5-style `rvsim.core`, `rvsim.cpu`,
    `rvsim.memory` and `rvsim.devices` modules.
  - Stats are read by path; the flat names (`dcache_misses`,
    `branch_accuracy_pct`, `stalls_data`, ...) are gone.
  - Unknown configuration keys raise `ValueError`, and `vlen` is checked
    when the configuration is read.
  - `MemoryController.Simple` takes a `latency` and keyword-only arguments;
    `MemoryController.DRAM` no longer takes `row_miss_latency`.
  - The package is split into `rvsim.config`, `rvsim.session` and
    `rvsim.cli`; the names exported from `rvsim` are kept.
- **Stats.** Several stats changed meaning or were removed; see the
  entries marked **Breaking (stats)** below.
- **Rust API.** `rvsim-core` exports `Simulator`, `common`, `isa`,
  `config`, `arch` and `stats`; the model's modules are crate-private.
  The crate directories are now `crates/rvsim-core` and
  `crates/rvsim-bindings`.
- **Timing.** Most workloads take a different number of cycles than in
  v1.2: the memory system, pipelines and predictors were rebuilt to follow
  real cores and gem5.
- **Repository layout.** The test runners moved to `tests/conformance`,
  and the scripts to `examples/analysis` and `tools/`.

## Changes in detail

These are the changes recorded as they were made during the release cycle,
after the features above were in place.

- `Simulator.audit_caches = True` (Rust: `Simulator::set_audit_caches`)
  checks every cache invariant after every event and ends the run with
  `SimError::CacheInvariant` at the first one broken; off by default, at
  no cost. It found the three bugs below.
- An exclusive L2 kept a copy of every line it fetched for the L1s and
  could prefetch a line an L1 held, so lines sat in both levels. It now
  hands such a line up without keeping it, keeps a shadow tag so it does
  not prefetch it, and installs it when the L1 evicts it; the L1I hands
  its clean victims down under the exclusive policy too.
- A cache's writeback buffer could hold more evictions than
  `write_buffers`: a fill installed its dirty victim whether or not a slot
  was free, and probe writebacks took eviction slots. A fill whose dirty
  victim finds the buffer full now waits for a slot, and a line a probe or
  back-invalidation demands goes back on the snoop-response path without
  taking one. A few cache-thrashing workloads take up to 3% more cycles.
- A dirty writeback and a snoop answered with a modified line carried
  only a header across the coherence interconnect, so they took one cycle
  and their line was missing from `coherence.interconnect.bytes`. Both now
  carry the line, and the snoop answer travels on the data channel, as
  CHI's `SnpRespData` does. Multicore timing changes by a few percent.
- **Breaking (stats).** `cache.l1d.exclusive_swaps` is removed: it was
  never counted, and what it described is `cache.l1d.writebacks` under
  the exclusive policy. A cache's `back_invalidations` and
  `coherence.invalidations`/`.downgrades` counted every such request,
  even for a line no copy of which was held; they now count only lines
  this cache or one above it held, and `probes` and `coherence.snoops`
  still count every request.
- **Breaking (stats).** `fu.util.<unit>` counted one per instruction that
  completed on a unit type, under a description of busy cycles; it now
  counts busy cycles at issue (one per instruction on a pipelined unit,
  the latency on an unpipelined one) including instructions later
  squashed, and the in-order backend counts memory ops, which it missed.
- **Breaking (stats).** `mdp.*` mirrored the predictor's lifetime totals,
  so after a stats reset they jumped back to them; they now count from the
  reset like every other stat. `wcb.coalesces` also counted a store that
  took an empty entry; it now counts only stores merged into a line the
  buffer held. `lsq.rescheduled_mem_ops` counted every cycle an op waited
  in memory1; it now counts each wait once.
- **Breaking (stats).** `pipeline.flushes.*` missed every flush commit
  takes (traps, interrupts, xRET, FENCE.I, SFENCE.VMA and WFI refetches,
  LR/AMO re-execution) and put coherence squashes under no cause. Each
  flush now counts once in `flushes.total` and once under one cause, with
  new `flushes.trap` and `flushes.coherence`; `flushes.squashed_insns`
  counts the ROB entries every flush drops on both backends (the in-order
  backend counted none). `pipeline.stalls.data` also counted cycles issue
  held for program order, and on the in-order backend cycles already
  counted as `stalls.fu_structural`; it now counts only operand waits,
  and a new `pipeline.stalls.ordering` counts the rest. The in-order
  backend now counts `stalls.backpressure`.
- **Breaking (stats).** `commit.op.load` counted LR, SC and every AMO;
  they now count in a new `commit.op.atomic`. `commit.vec.misc` counted
  the Zvbb/Zvbc bit-manipulation and Zvk* crypto ops: bit-manipulation now
  counts in `commit.vec.int` and crypto in a new `commit.vec.crypto`, so
  `misc` is permute, mask and configuration as described.
- **Breaking (stats).** A cache's `prefetches.useful` counted demand
  requests that joined a prefetch still in flight; that count is now
  `prefetches.late`. `prefetches.useful` counts prefetched lines a request
  found once installed, `prefetches.unused` prefetched lines dropped before
  any request found them (as gem5's `pfUseful` and `pfUnused` do), and the
  derived `prefetches.used` (late + useful) and `prefetches.accuracy`
  (used / issued) follow Feedback Directed Prefetching (Srinath et al.,
  HPCA 2007); gem5's `pfLate` and `accuracy` are defined differently.
- A program's exit no longer drops the stores it committed just before
  exiting: they finish writing, so the console shows everything printed
  (`fib.elf` on `presets.p550()` used to end at `fib(20)=`). The cycles
  spent finishing them are not counted; `cycles` and the stats window end
  at the exit instruction, as before.
- Prefetchers follow the published hardware (decision 14, after the
  Cortex-A72 TRM §6.4.9 and Intel's optimization manual).
  `Config(load_prefetcher=LoadPrefetcher.Stride(...))` is a load
  prefetcher in the load/store unit: a PC-indexed stride table trained on
  virtual addresses that keeps each stream `l1_lines` ahead in the L1D and
  `l2_lines` ahead in the L2, and at a page boundary stops
  (`PageBoundary.Stop()`, at the page's real size) or continues through
  the data TLB (`PageBoundary.CrossWithTlb()`).
  `Config(store_prefetcher=StorePrefetcher.Stream(...))` prefetches runs of
  L1D store misses into the L2. `presets.cortex_a72()` uses both with the
  A72's reset values; `presets.p550()` uses a load prefetcher that keeps to
  the page, its prefetchers being unpublished. Cache-side prefetchers now
  keep to the 4 KiB page of the access that triggered them, the
  cache-side stride prefetcher is indexed by the load's PC (it used to be
  indexed by the accessed line, so a stride of a line or more never
  trained) and issues whole lines. New stats: `core<N>.prefetch.loads.*`
  and the caches' `prefetches.page_crossing`, `.dropped` and
  `.store_stream`.
- `pipeline.stalls.control` counts cycles, as its unit always said: each
  cycle from a backend redirect (a misprediction, trap or re-execution)
  until rename hands on the first instruction from the new path. It used
  to count squashes, duplicating `pipeline.flushes.total`.
- `MemoryController.Simple` takes a `latency` in core cycles (default 120),
  and its arguments are keyword-only. `MemoryController.DRAM` no longer
  takes `row_miss_latency`, which it never used: its row-miss cost is
  `t_pre + t_ras + t_cas`. In the Rust configuration `memory.row_miss_latency`
  is renamed `memory.simple_latency`.
- `rvsim --preset` and `rvsim bench --preset` accept every preset in
  `rvsim.presets.PRESETS` (`cortex_a72`, `m1` and `p550` as well as
  `basic` and `fast`) instead of a hard-coded `basic` or `fast`.
- The documentation describes the simulator as it is: a design page and
  ADRs 9 to 13 on the state split, semantics versus timing, perform-point
  memory, following real cores and split stores; every configuration
  parameter with its default; the full ISA and CSR set; the sim-control
  device and boot loader; a catalogue of every stat path and how to
  measure a region; `rvsim bench`; and the `fast()` and `linux()`
  presets. Claims that had drifted from the code (TLB sizes, a prefetch
  filter that does not exist, the DRAM row-miss cost, a builder API, the
  in-order backend's width) are corrected.
- The out-of-order backend issues a plain scalar store in two halves, as
  real out-of-order cores do: the address as soon as the base register is
  ready, the data when its value is. A younger load that waits for older
  stores' addresses no longer waits for a store's data; a load of the same
  address waits for the data and then forwards it, and commit retires a
  store only once its data has arrived. A store whose operands are both
  ready issues whole, as before. `lsq.split_stores` counts the stores that
  split. gem5's O3 does not split stores, so store-heavy kernels now run
  faster than in gem5.
- `presets.p550()` and `presets.cortex_a72()` follow the measured
  hardware: the P550's 4-way L1s, 3-cycle L1D load-to-use, 13-cycle L2,
  38-cycle L3, 194 ns memory, 4-cycle FP units, one load and one store
  AGU, 32-entry L1 TLBs and 512-entry L2 TLB, misaligned-access trap and
  1.4 GHz clock; the A72's 4-cycle L1D, 21-cycle L2, 162 ns memory, two
  integer ALUs with one multiply pipe, 32-entry load and 16-entry store
  queues, 31-entry return stack, 32/1024-entry TLBs and 1.5 GHz clock.
  Each value cites its source in the preset. The P550 preset no longer
  pins the stack at 1 MiB, where programs with large static data overran
  it. `tools/diag/latency_probe.py` measures a preset's load-to-use
  latency at each cache level with a pointer chase, which is how the
  values were set.
- The out-of-order backend's rename allocates into the ROB, issue-queue,
  load-queue and store-buffer entries that were free at the end of the
  previous cycle, as pipelined allocation bookkeeping does, instead of
  entries commit freed in the same cycle.
- The out-of-order backend recovers from a squash as gem5's O3 does:
  fetch spends the redirect cycle squashing and fetches the target the
  cycle after; commit drains the flushed ROB entries at
  `Backend.OutOfOrder(squash_width=8)` per cycle (gem5's `squashWidth`)
  and rename resumes the cycle after it finishes; the rename map is
  restored at once. It drained them at the pipeline width and charged a
  further rename-map rebuild walk when no checkpoint matched, which put
  the correct path's execution three cycles behind gem5's after every
  misprediction. `pipeline.stalls.rename_rebuild` is gone.
- `Config(store_forward_latency=N)` sets the cycles a load forwarded from
  the store buffer takes to reach writeback, where a load the L1D answers
  takes the L1D hit latency; unset keeps the L1D hit latency, `1` is
  gem5's O3 LSQ, and `0` writes the load back in the cycle it matches.
  The gem5 comparison runs at `1`.
- **Breaking (Rust API).** `rvsim-core`'s model is crate-private: the
  `exec`, `sim`, `soc` and `uarch` modules, `SystemState`, `Uncore`,
  `CoreCtx` and `StageCtx` are no longer exported. The crate's interface
  is `Simulator` (loading, running, harts, CSRs, translation, stats,
  trace, console and a plain-data `system::snapshot::PipelineSnapshot`),
  the `common`, `isa`, `config` and `arch` modules, and the statistics
  tree at `rvsim_core::stats`. Items the model had stopped using are
  gone with the change. See design decision 8. No behaviour change.
- `rvsim-core` classifies vector ops by execution unit: `VectorOp::class`
  yields a `VecClass` whose narrow op types (`VecAluOp`, `ReduceOp`,
  `MaskOp`, `PermuteOp`, `CryptoOp`) the vector executors take, and
  `VectorOp::VSlideUp` / `VSlideDown` carry their `SlideOffset`. The
  executors' entry points changed accordingly. Rename reserves
  physical registers and checkpoint slots before it commits to an
  instruction instead of re-checking afterwards. No behaviour change.
- The API reference at `docs/api.md` is generated from the package's
  docstrings, including the compiled extension's, by mkdocstrings; the
  extension's classes report `rvsim._core` as their module.
- The core no longer prints to stdout or stderr. Device, loader and
  HTIF messages are `tracing` events (targets `rvsim::syscon`,
  `rvsim::loader`, `rvsim::htif`, `rvsim::dma`), shown with
  `RUST_LOG`; `Hart` and the general-purpose register file implement
  `Display` in place of the `dump` helpers.
- A configuration key the core does not know is refused with
  `ValueError` instead of being ignored; `rvsim/_core.pyi` is checked
  against the built extension by `make test-python`, and now lists
  `Simulator.stats_between`.
- `rvsim._core.version()` reports the built version instead of a
  hard-coded `0.1.0`; a session's cached checkpoints record it.
- The published x86-64 Linux wheel runs on any x86-64 CPU. The previous
  wheels were built with `-C target-cpu=native` from a committed cargo
  config and needed the build machine's AVX2, BMI2 and FMA.
- `vlen` is checked as the configuration is read: a value that is not a
  power of two in `[128, 2048]` raises `ValueError`. Every hart has its
  vector registers from construction; `rvsim-core`'s `PipelineConfig::vlen`
  is a `Vlen`, `RegisterFile::new` takes it, and the `Option` around the
  vector register file is gone.
- A panic inside the simulator raises `pyo3_runtime.PanicException` in
  Python instead of aborting the interpreter: release builds unwind.
- `rvsim-core` owns RAM in one place: `sim::memory::Ram` is the zeroed
  image the `GlobalMemory` holds, and every reader and writer (the memory
  controllers, virtio DMA, instruction fetch, the loader, host probes and
  checkpoints) goes through it. The raw-pointer `RamRegion` and
  `DramBuffer` types, `Bus::ram_region`, `Bus::load_binary_at` and
  `Device::take_dma_writes` are gone; `Device::drain` takes the memory,
  `VirtioBlock::new` takes only its MMIO base, and the memory controllers
  no longer take a buffer. A DMA write is noted in the reservation set
  and write log as it lands. `Simulator` and `SystemState` are `Send` and
  `Sync` by construction rather than by unchecked `unsafe impl`s.
- `rvsim.presets` gains `cortex_a72()`, `m1()` and `p550()`, replacing the
  machine configs under `scripts/benchmarks`.
- Comparisons (`Result.compare`, `Sweep.run().compare`, `Stats.tabulate`),
  `rvsim --watch` and the analysis examples read stats by path
  (`core0.cache.l1d.misses`, `core0.bp.committed.accuracy`). The flat names
  they used (`dcache_misses`, `branch_accuracy_pct`, `stalls_data`) no longer
  exist, so those tables printed empty cells and `--watch` failed.
  `Stats.from_core` reports whole-number counters as ints.
- Configs that use vector functional units can be pickled, so `Sweep` runs
  them in parallel.
- Repository layout: the test runners moved from `testing/` to
  `tests/conformance`, the analysis scripts to `examples/analysis`, and the
  gem5 comparison, baseline recorders and Linux boot driver to `tools/`.
  The Python package is split into `rvsim.config`, `rvsim.session` and
  `rvsim.cli`; the names exported from `rvsim` are unchanged.
- `rvsim-core` modules are layered (`common`, `isa`, `config`, `arch`,
  `exec`, `sim`, `soc`, `uarch`, `system`), each depending only on the ones
  before it. Rust paths into the crate have changed. At the crate root,
  `SimState` is now `SystemState` and `SharedState` is `Uncore`; the other
  re-exports are unchanged.
- Multi-core systems: `Config(hart_count=N)` builds N cores with private
  caches, per-hart CLINT/PLIC contexts and device-tree entries; bare-metal
  programs start every hart with its id in `a0`.
- Coherence fabric: MESI states in the private caches, a home agent
  (`HomeAgent.SnoopFilter` or `HomeAgent.Broadcast`) at the LLC and an
  interconnect (`Interconnect.Crossbar`, `Ring`, `Mesh`, `Torus`,
  `Hypercube`), configured with `Config(coherence=Coherence(...))`;
  reported under `coherence.*`.
- Non-blocking caches: MSHRs with coalescing, a writeback buffer, real
  prefetch fetches and honoured inclusion policies; dirty lines reach DRAM.
- `cpu.harts[i]` exposes every hart's `pc`, `privilege`, `regs` and `csrs`;
  `rvsim prog.elf --harts N` on the command line.
- Statistics are rooted at `core<N>` and `hart<N>`, with `system.*` sums.
- Tracing: every event is tagged with its hart, and `cpu.trace_filter(harts=,
  cycles=, trap_causes=)` narrows an armed trace to some harts, a cycle
  window and specific `mcause` values.
- Checkpoints save and restore every hart and the cycle counter; `save`
  drains the pipelines first so the checkpoint is the committed state.
- Device DMA writes are published to the reservation set and write log;
  the MMU is per core; BTB geometries that are not a power of two of sets
  are rejected.
