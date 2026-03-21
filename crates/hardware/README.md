# rvsim-core

[![crates.io](https://img.shields.io/crates/v/rvsim-core)](https://crates.io/crates/rvsim-core)
[![docs.rs](https://docs.rs/rvsim-core/badge.svg)](https://docs.rs/rvsim-core)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

Cycle-accurate RISC-V 64-bit system simulator core. This is the Rust library that powers [`rvsim`](https://pypi.org/project/rvsim/).

## What is this?

`rvsim-core` is a hardware-level RV64IMAFDC simulator that models a complete SoC at cycle granularity. It implements two pluggable microarchitectural backends (out-of-order superscalar and in-order scalar) sharing a common frontend, memory hierarchy, and SoC device layer. It boots Linux 6.6 through OpenSBI to a BusyBox shell and passes all 134/134 `riscv-tests`.

For the Python API and high-level usage, see the [main repository](https://github.com/willmccallion/rvsim).

## Architecture

### Pipeline

Two execution backends behind a shared frontend (Fetch1 / Fetch2+Decode / Rename):

**Out-of-Order (10-stage superscalar):**
- Physical register file with free list and dual rename maps (speculative + committed)
- CAM-style issue queue with wakeup/select, oldest-first priority, per-type port limits
- Reorder buffer (circular buffer, O(1) tag lookup) for in-order commit with precise exceptions
- Load queue for memory ordering violation detection and replay
- Configurable functional unit pool (counts and latencies per type)

**In-Order (scalar):**
- Scoreboard-based operand tracking with tag bypass from ROB entries
- FIFO issue queue with head-of-queue blocking
- Backpressure gating via inter-stage latch occupancy

**Shared across both backends:**
- Commit stage with CSR serialization, FENCE ordering, SFENCE.VMA store-drain semantics, LR/SC reservation handling
- Memory1 (D-TLB + L1D tag probe), Memory2 (L1D data + store-to-load forwarding), Writeback
- Store buffer with forwarding (full/partial overlap), speculative drain, write-combining buffer
- Issue-time serialization: system/CSR instructions wait for all older completions, FENCE waits for matching pred operations, loads/stores blocked by in-flight FENCE, loads wait for older store address resolution

### Memory Hierarchy

- **SV39 MMU**: separate iTLB/dTLB (32 entries), shared L2 TLB (512 entries, 4-way), hardware page table walker with A/D bit management
- **L1i / L1d / L2 / L3 caches**: configurable size, associativity, latency, replacement policy (LRU, PLRU, FIFO, Random, MRU)
- **Non-blocking L1D** via MSHRs with request coalescing
- **Prefetchers**: next-line, stride, stream, tagged (per cache level)
- **Inclusion policies**: non-inclusive, inclusive (back-invalidation), exclusive (L1-L2 swap)
- **DRAM controller**: row-buffer aware timing (tCAS, tRAS, tPRE, row-miss, tRRD, tREFI/tRFC, bank interleaving)

### Branch Prediction

Five pluggable predictors: Static, GShare, Tournament, Perceptron, TAGE (tagged geometric history length with loop predictor). Shared BTB (set-associative), RAS (snapshot/restore), and arbitrary-length GHR with speculative update and repair.

RAS recognizes both x1 and x5 as link registers per RISC-V spec Table 2.1, including coroutine swap detection.

### ISA

**RV64IMAFDC** — base integer, multiply/divide, atomics (LR/SC + AMO), single/double-precision float with IEEE 754 NaN-boxing, compressed instructions. Full privileged architecture: M/S/U modes, CSRs, trap delegation, MRET/SRET, WFI, SFENCE.VMA, FENCE/FENCE.I, PMP (16 regions).

### SoC Devices

CLINT (timer), PLIC (external interrupts), 16550A UART, VirtIO MMIO block device, Goldfish RTC, SYSCON (poweroff/reboot), HTIF (test interface). Auto-generated device tree blob.

## Configuration

All parameters are runtime-configurable via the `Config` struct (JSON-serializable via serde):

```rust
use rvsim_core::config::{Config, BackendType, BranchPredictor};

let mut config = Config::default();
config.pipeline.backend = BackendType::OutOfOrder;
config.pipeline.width = 4;
config.pipeline.rob_size = 128;
config.pipeline.branch_predictor = BranchPredictor::Tage;
config.caches.l1_d.size_bytes = 32768;
config.caches.l1_d.ways = 8;
```

## Features

- `commit-log` — enables per-instruction commit logging for trace-driven analysis
- `always-trace` — enables pipeline tracing macros unconditionally (normally compiled out)

## License

Licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
