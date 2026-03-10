# RVSim Roadmap: From Current State to Research-Grade Simulator

This is an ordered execution plan. Items within each phase can be done in any order, but **complete all items in a phase before moving to the next**. Each phase ends with a concrete testing checkpoint.

---

## Phase 0: Engineering Excellence (Modern Rust Standards) **IN PROGRESS**

Before scaling the simulator, we must ensure the codebase is robust, idiomatic, and maintainable. This sets the "highest standard" for a professional Rust project.

### 0.1 Strict Linting & Formatting
- [x] Enable `clippy::pedantic`, `nursery`, and `cargo` groups in `Cargo.toml`.
- [x] Configure `rustdoc` lints to ensure high-quality documentation.
- [x] Add `rustfmt.toml` for consistent project-wide formatting.
- [x] Resolve remaining `unwrap()` calls in core logic — zero `unwrap_used`/`expect_used` in production; full `make lint` passes clean.

### 0.2 Crate Metadata & Documentation
- [x] Add comprehensive workspace metadata (categories, keywords, readme).
- [x] Configure `docs.rs` metadata for professional online documentation.
- [ ] **Educational Foundation**: Initialize `mdBook` in `/book` for the "Architecture Guide."

### 0.3 Performance & Safety "Hardening"
- [x] Implement advanced release profiles (LTO, codegen-units, panic=abort).
- [x] **Newtype Pattern**: `VirtAddr`, `PhysAddr`, `RobTag`, `PhysReg` already exist; `RegIdx`, `CsrAddr`, `Cycle` added.
- [x] **Custom Error Types**: `SimError` enum (`thiserror`-based) replaces `Result<_, String>` in `pre_tick` / `tick`; `load_binary` uses `Result<_, SimError>` instead of `process::exit`.

---

## Phase 1: Functional Correctness (Boot bare-metal programs reliably) **COMPLETE**

### 1.1 Zero BSS segments in the ELF loader
### 1.2 Move CSR writes from execute to commit (O3 backend)
### 1.3 Fix in-order backend FP exception flags
### 1.4 Fix cache flush writeback
### 1.5 Fix compressed instruction HINT encodings
### 1.6 FMA rounding mode fixes
### 1.7 Add FP underflow detection
### 1.8 VirtIO write completion fixes

---

## Phase 2: Privileged Architecture (Boot OpenSBI + Linux)

### 2.1 Enforce PMP in the memory access path
### 2.2 Enforce MCOUNTEREN/SCOUNTEREN
### 2.3 Fix SIP read/write semantics
### 2.4 Fix FENCE enforcement
### 2.5 Move branch predictor updates to commit time
### 2.6 Fix PLIC claim/complete semantics
### 2.7 Fix PLIC sub-word writes
### 2.8 Generate access faults for unmapped regions
### 2.9 DTB generation

---

## Phase 3: Timing Accuracy & Structured Tracing

### 3.1 Structured Event-Based Tracing (Architecture Overhaul)
- **Problem**: Current `eprintln!` tracing is a "wall of text," slow, and hard to grep.
- **Goal**: Implement a `TraceEvent` enum and a `Tracer` trait.
- **Implementations**:
    - `LogTracer`: Clean, formatted text for terminal debugging.
    - `JsonTracer`: Structured output for the web-frontend and post-processing tools.
    - `EventBus`: A high-performance internal bus to distribute events to stats/trace/visualizers.

### 3.2 Branch prediction pipeline latency (1-2 cycles)
### 3.3 Fix RAS circular buffer
### 3.4 Set-associative L1 TLBs (4-way)
### 3.5 Tree-based PLRU algorithm
### 3.6 DRAM timing (tRCD, tWR, tWTR, tCCD)
### 3.7 Atomic operation latency
### 3.8 Store drain rate under pressure
### 3.9 PLIC >64 sources support

---

## Phase 4: DRAM Subsystem (DDR4/5 ready)

### 4.1 XOR-based bank indexing
### 4.2 Bank group modeling (DDR4/5 differentiator)
### 4.3 Burst-level timing (BL8/BL16)
### 4.4 FR-FCFS Command bus scheduling

---

## Phase 5: Polish for Publication

### 5.1 Fix ROB tag wraparound
### 5.2 Speculative load wakeup race condition
### 5.3 Write-combining buffer integration
### 5.4 MSHR entry squashing
### 5.5 Simulation speed: replace `Vec::remove(0)` with `VecDeque`
### 5.6 UART FCR (FIFO) implementation
### 5.7 MESI Coherence states (foundation for multicore)
### 5.8 DTB auto-generation from SoC config

---

## Phase 6: Educational Web Vision (The "Interact" Layer)

Transform `rvsim` into an interactive educational platform for computer architecture.

### 6.1 WebAssembly (WASM) Port
- Port the `rvsim-core` to WASM using `wasm-bindgen`.
- Optimize the memory interface to minimize boundary crossings between JS and WASM.

### 6.2 Interactive Visualizers
- **Pipeline View**: Real-time visualization of instructions moving through Fetch -> Commit.
- **Cache Heatmap**: Visualize hits/misses/evictions on a grid representing sets and ways.
- **Branch History**: Show the GShare/TAGE history tables and how they adapt to patterns.

### 6.3 "The Processor Lab"
- Create a series of "Lab" pages in the website/book.
- **Lab 1: The Matrix**: Demonstrate cache thrashing and how tiling/blocking fixes it.
- **Lab 2: Predictability**: Show how a simple `if` inside a loop can destroy performance if not sorted.
- **Lab 3: Data Hazards**: Visualize "Bubbles" in the pipeline from load-use dependencies.

---

## After all this, is it "done"?

After Phase 6, this will be a simulator that is not only a **research-grade tool** (Phase 1-5) but also the **premier educational resource** for modern computer architecture (Phase 6).
