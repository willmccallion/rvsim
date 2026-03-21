# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-03-20

Initial stable release. Both out-of-order and in-order backends boot Linux 6.6 through OpenSBI to a BusyBox shell. All 134/134 riscv-tests pass on both backends.

### Pipeline - Out-of-Order Backend

- 10-stage superscalar pipeline: Fetch1, Fetch2/Decode, Rename, Issue, Execute, Mem1, Mem2, Writeback, Commit
- Physical register file (PRF) with free list, speculative and committed rename maps
- CAM-style issue queue with wakeup/select, oldest-first priority, configurable width
- Per-type functional unit port limits (load ports, store ports)
- Configurable FU pool: IntALU, IntMul, FpAdd, FpMul, FpFma, FpDiv, Branch, Mem — counts and latencies per type
- Reorder buffer (circular buffer with HashMap tag index for O(1) lookup)
- Precise exceptions via ROB in-order commit
- Load queue for memory ordering violation detection and replay
- Store buffer with store-to-load forwarding (full and partial overlap detection)
- Write-combining buffer for store coalescing
- Branch misprediction recovery: GHR repair from snapshot, RAS restore, rename map rebuild from committed state + surviving ROB entries
- Speculative load wakeup on MSHR availability
- Partial pipeline flush (flush after mispredicting instruction's ROB tag, keep older in-flight work)

### Pipeline - In-Order Backend

- Scalar pipeline sharing the same frontend and commit/memory/writeback stages as O3
- Scoreboard-based operand tracking with tag bypass from completed ROB entries
- FIFO issue queue with head-of-queue blocking
- Backpressure gating via execute-to-memory1 latch occupancy
- Issue-time serialization checks matching O3 behavior:
  - System/CSR instructions wait for all older instructions to complete
  - FENCE waits for older operations matching predecessor bits
  - Loads/stores blocked by older in-flight FENCE with matching successor bits
  - Loads wait for all older stores to resolve addresses

### Pipeline - Shared Stages

- Commit stage: CSR write serialization, FENCE store-drain semantics, SFENCE.VMA deferred TLB flush (waits for store buffer drain), FENCE.I deferred I-cache invalidation, SATP write with store-drain and redirect, MRET/SRET privilege return, LR/SC reservation check at commit time with SC failure recovery
- Memory1: D-TLB translation, L1D tag probe, MSHR allocation for misses (loads parked, stores proceed), fault propagation
- Memory2: L1D data access, store-to-load forwarding from store buffer, NaN-boxing for FP loads, LR/SC/AMO handling, store buffer address resolution
- Writeback: result selection (load data vs jump link vs ALU), ROB completion, FP flags and PTE update propagation

### Memory Hierarchy

- SV39 virtual memory with separate iTLB and dTLB (fully-associative, configurable size)
- Shared L2 TLB (set-associative, configurable size/ways/latency)
- Full hardware page table walker with accessed/dirty bit management
- L1-I, L1-D, L2, L3 caches — independently configurable size, associativity, latency
- Cache replacement policies: LRU, Pseudo-LRU (tree-based), FIFO, Random, MRU
- Non-blocking L1D via Miss Status Holding Registers (MSHRs) with request coalescing
- Hardware prefetchers per cache level: next-line, stride (PC-indexed), stream (sequential detection), tagged (prefetch-on-prefetch)
- Prefetch deduplication filter to avoid redundant requests
- Cache inclusion policies: non-inclusive (default), inclusive (back-invalidation on eviction), exclusive (L1-L2 swap on eviction)
- Write-combining buffer for store coalescing before L1D drain
- DRAM controller with row-buffer aware timing: tCAS, tRAS, tPRE, row-miss latency, tRRD (bank-to-bank), tREFI/tRFC (refresh), configurable bank count and row size

### Branch Prediction

- Five pluggable predictors:
  - Static (always not-taken)
  - GShare (global history XOR PC, 2-bit saturating counters)
  - Tournament (local + global two-level adaptive with meta-predictor)
  - Perceptron (neural predictor with configurable history length and table size)
  - TAGE (Tagged Geometric History Length with 8 tagged tables, loop predictor, configurable history lengths and tag widths)
- Branch Target Buffer (BTB): set-associative, configurable entries and ways
- Return Address Stack (RAS): circular buffer with snapshot pointer for speculative recovery
- Global History Register (GHR): arbitrary-length bit vector with speculative update and repair from per-instruction snapshots
- RAS link register detection per RISC-V spec Table 2.1: both x1 (ra) and x5 (t0) recognized as link registers, with coroutine swap detection (pop-then-push when rd and rs1 are different link registers)

### ISA

- RV64I: full base integer instruction set including W-variants (32-bit operations with sign extension)
- M extension: MUL, MULH, MULHSU, MULHU, DIV, DIVU, REM, REMU (+ W variants)
- A extension: LR/SC with forward progress guarantee, AMO operations (SWAP, ADD, AND, OR, XOR, MIN, MAX, MINU, MAXU) for word and doubleword
- F extension: single-precision IEEE 754 — arithmetic, FMA, comparisons, conversions (int-to-float, float-to-int), classification, sign injection, moves, NaN-boxing validation per spec section 12.2
- D extension: double-precision IEEE 754 with full parity to F
- C extension: compressed (16-bit) instruction encoding, expanded to 32-bit equivalents at decode time
- Privileged architecture: M/S/U privilege modes, full CSR set, trap delegation (medeleg/mideleg), MRET/SRET, WFI, ECALL/EBREAK
- SFENCE.VMA with ASID-aware and address-specific TLB invalidation (deferred to commit after store drain)
- FENCE with predecessor/successor ordering bits, FENCE.I with deferred I-cache invalidation
- Physical Memory Protection (PMP): 16 regions with TOR/NAPOT/NA4 address matching
- Counter CSRs: CYCLE, TIME, INSTRET with mcounteren/scounteren access control
- FP CSRs: frm (rounding mode), fflags (exception flags), fcsr; flags accumulated from in-flight pipeline entries for CSR reads
- mstatus privilege control bits: TSR (trap SRET), TW (timeout WFI), TVM (trap virtual memory), FS (FP state)

### SoC

- CLINT (Core Local Interruptor): mtime/mtimecmp timer interrupt with configurable clock divider
- PLIC (Platform-Level Interrupt Controller): 53 interrupt sources, 2 contexts (M-mode and S-mode), priority-based arbitration, claim/complete protocol
- UART: 16550A-compatible with interrupt support, configurable output (stdout, stderr, quiet)
- VirtIO MMIO block device: virtqueue-based DMA with interrupt notification, filesystem-backed storage
- Goldfish RTC: real-time clock for wall-clock time (used by Linux for system time initialization)
- SYSCON: system controller for poweroff and reboot signals
- HTIF (Host-Target Interface): tohost/fromhost protocol for riscv-tests pass/fail detection and syscall proxying
- Auto-generated Flattened Device Tree (FDT/DTB) synthesized from active configuration

### Python API

- PyO3-based native extension module (`rvsim._core`)
- `Config`: composable configuration with `Backend`, `BranchPredictor`, `Cache`, `Prefetcher`, `MemoryController`, `Fu` builders
- `Environment`: high-level run-to-completion with stats collection
- `Simulator`: low-level tick-by-tick control with `run_until(pc=..., privilege=...)`
- `Sweep`: parallel multi-configuration benchmarking across CPU cores
- Register and CSR access by name (`reg.A0`, `csr.MSTATUS`)
- Memory access views (`cpu.mem8`, `cpu.mem16`, `cpu.mem32`, `cpu.mem64`)
- Pipeline snapshot visualization
- Checkpoint save/restore
- Stats querying with regex filtering and tabulation

### Statistics

- Cycle accounting: retiring, ROB-empty, ROB-stall, WFI, per-cycle retirement histogram (0/1/2/3+)
- Privilege mode breakdown: U/S/M mode cycles
- Pipeline stalls: memory, control, data hazard, FU structural, backpressure, MSHR-full, dispatch
- Branch prediction: committed and speculative accuracy and misprediction counts
- Pipeline flushes: total, branch-caused, system-caused, squashed instruction count, memory ordering violations
- Cache hierarchy: per-level access counts, hits, miss rates
- Memory subsystem: MSHR allocations/coalesces/full-stalls, load replays
- Inclusion tracking: back-invalidations, exclusive L1-to-L2 swaps
- Write-combining buffer: coalesces and drains
- Prefetch filter: dedup counts per cache level (L1/L2/L3)
- Instruction mix: ALU, load, store, branch, system, FP (broken down into load/store/arith/FMA/div-sqrt)
- FU utilization: per-unit-type busy cycle counts

### Analysis Scripts

- `width_scaling.py`: IPC vs superscalar width across workloads
- `branch_predict.py`: predictor accuracy comparison
- `cache_sweep.py`: L1D size vs miss rate
- `inst_mix.py`: instruction class breakdown
- `stall_breakdown.py`: stall cycle attribution
- `top_down.py`: top-down microarchitecture analysis
- `o3_inorder.py`: O3 vs in-order IPC comparison
- `design_space.py`: multi-dimensional design-space sweep

### Build and Test

- Rust 2024 edition with strict linting: clippy pedantic + nursery + cargo, `#[deny(unwrap_used, expect_used, todo, unimplemented)]`
- 1581 tests (1415 unit + 166 integration) + 8 doctests
- Zero clippy warnings across all targets
- Dev profile at opt-level 1 (usable simulation speed during development), release with fat LTO and single codegen unit
- Maturin-based Python packaging with PyO3 stable ABI (abi3-py310)

[1.0.0]: https://github.com/willmccallion/rvsim/releases/tag/v1.0.0
