# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

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

## Releases

- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
