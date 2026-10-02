# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

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
