# RVSim Roadmap: From Current State to Research-Grade Simulator

This is an ordered execution plan. Items within each phase can be done in any order, but **complete all items in a phase before moving to the next**. Each phase ends with a concrete testing checkpoint.

---

## Phase 1: Functional Correctness (Boot bare-metal programs reliably) **COMPLETE**

These are bugs that produce **wrong architectural state**. Fix these first because nothing else matters if the ISA semantics are wrong. You can validate each fix with riscv-tests or custom bare-metal programs.

### 1.1 Zero BSS segments in the ELF loader
`sim/loader.rs:125-137`: When `p_memsz > p_filesz`, the gap (BSS) is not zeroed. Every C program with uninitialized globals will behave unpredictably.

### 1.2 Move CSR writes from execute to commit (O3 backend)
`o3/mod.rs:416-431`: CSR writes are applied at execute-completion time. If an older instruction traps, the pipeline flushes but the CSR retains its speculatively-applied value. Move all CSR writes to commit.

### 1.3 Fix in-order backend FP exception flags (apply at commit, not execute)
`inorder/execute.rs:496-505`: FP flags are applied directly to `cpu.csrs.fflags` during execute. If an older instruction traps, the flags from the younger FP instruction persist. The O3 backend already does this correctly via the ROB — apply the same pattern to in-order.

### 1.4 Fix cache flush to write back dirty lines before invalidating
`cache/mod.rs:522-532`: `flush()` clears dirty+valid bits but never writes data back. FENCE.I and SFENCE.VMA depend on dirty data being visible after flush. Without writeback, self-modifying code and page table updates silently lose data.

### 1.5 Fix compressed instruction HINT encodings
`rvc/expand.rs`: C.ADDIW, C.SLLI, C.LWSP, C.LDSP return 0 (illegal instruction) when `rd == 0`. These should expand to valid NOPs/HINTs per the C extension spec. Some toolchains emit these.

### 1.6 FMA rounding mode: use RISC-V rounding mode, not host
`fpu/mod.rs:636-639,713-716`: FMA uses Rust's `mul_add()` with the host rounding mode. When `execute_with_rm()` is called with RTZ/RDN/RUP/RMM, FMA falls through to `execute()` losing the rounding mode. Fix the FMA path to respect the explicit rounding mode.

### 1.7 Add FP underflow exception detection
`fpu/mod.rs:34-35`: `FE_UNDERFLOW` is defined but never mapped to `FpFlags::UF` in `read_host_fp_flags()`. Map it so fflags reflects underflow correctly.

### 1.8 VirtIO write completion: report actual bytes written
`virtio_disk.rs:382-384`: Write operations set `len_written = 0` in the used ring. Set it to the actual byte count. Some guest drivers check this.

> **Checkpoint: Run riscv-tests (rv64ui, rv64um, rv64ua, rv64uf, rv64ud, rv64uc)**
> All ISA compliance tests should pass. Run with both the in-order and O3 backends. If you have any custom FP or atomic stress tests, run those too. This validates that your architectural state is correct for single instructions.

---

## Phase 2: Privileged Architecture (Boot OpenSBI + Linux)

These items are required for the M→S→U privilege transitions that OpenSBI and Linux depend on. Without these, the kernel will hang, fault on legal operations, or silently corrupt state.

### 2.1 Enforce PMP in the memory access path
`mmu/pmp.rs` has a complete PMP implementation but `pmp.check()` is never called. Add PMP checks to:
- `memory1.rs`: after `cpu.translate()` returns the physical address
- `memory2.rs`: before load/store/atomic execution
- `ptw.rs`: on every PTE read (spec requires PMP on PTW accesses)
- Fetch path: on instruction fetch physical addresses

OpenSBI configures PMP before handing off to the kernel. Without enforcement, the kernel runs with M-mode's full address space — which happens to work until it doesn't.

### 2.2 Enforce MCOUNTEREN/SCOUNTEREN on counter reads
`csr.rs:53-57`: Reading CYCLE/TIME/INSTRET from S/U-mode bypasses counter-enable checks. Trap to illegal instruction when the corresponding enable bit is not set. Linux uses this to control userspace access to `rdcycle`/`rdtime`.

### 2.3 Fix SIP read/write semantics
`csr.rs:156-159`: SIP writes correctly mask to SSIP only. Verify that SIP *reads* reflect all delegated interrupt pending bits (SSIP, STIP, SEIP) — not just the ones that are writable. The kernel reads SIP to check for pending timer and external interrupts.

### 2.4 Fix FENCE enforcement
`commit.rs:254-260`: FENCE only drains the store buffer when `pred.w` is set. The successor set is ignored entirely. At minimum:
- FENCE rw,rw: drain store buffer AND stall dispatch until all older loads/stores complete
- FENCE w,w: drain store buffer
- FENCE r,r: stall dispatch until all older loads complete
- FENCE.TSO: drain store buffer (store→load ordering)

The Linux kernel uses `FENCE rw,rw` for synchronization on every lock acquire/release. Getting this wrong will cause silent data corruption under concurrent access patterns even on single-core (interrupt handlers vs main code).

### 2.5 Move branch predictor updates to commit time
`execute.rs:169-173`: `update_branch()` is called at execute time, polluting tables with speculative/wrong-path data. Buffer the branch outcome in the ROB entry and call `update_branch()` during commit. This also fixes the wrong-path training problem (items 3+4 from the original audit).

### 2.6 Fix PLIC claim/complete semantics
`plic.rs:195-202`: Claim read immediately clears the pending bit. Per spec, pending should remain set until the completion write. Fix: clear pending on the completion write, not the claim read.

### 2.7 Fix PLIC sub-word write handling
`plic.rs` write_u8/write_u16: Offset is masked to 4-byte boundary, losing byte position. Fix the masking to preserve byte offset within the word.

### 2.8 Generate access faults for unmapped address accesses
`soc/interconnect.rs:236-241,268-271`: Reads return 0, writes are dropped silently. Return a bus error that the pipeline converts to a load/store access fault exception. Linux's device probing depends on faults from unmapped regions.

### 2.9 Implement DTB generation (or provide a working static DTB)
`sim/dtb.rs` is empty. Linux needs an accurate DTB describing memory, CLINT, PLIC, UART, VirtIO addresses and IRQ numbers. At minimum, provide a static DTB that matches your SoC layout. Ideally, generate it from the SoC config.

> **Checkpoint: Boot OpenSBI → Linux**
> 1. Build OpenSBI with your platform support (or use generic platform)
> 2. Load OpenSBI + Linux kernel image + DTB
> 3. OpenSBI should initialize, configure PMP, and jump to S-mode
> 4. Linux should print boot messages, mount initramfs, and reach a shell
> 5. Run basic commands: `ls`, `cat /proc/cpuinfo`, `echo hello > /tmp/test && cat /tmp/test`
> 6. If Linux hangs, use your PC trace to find where — common failure points are: PMP fault in early boot, missing timer interrupt, FENCE ordering in spinlocks, DTB mismatch

---

## Phase 3: Timing Accuracy (Match gem5 within ~10-15%)

Now that Linux boots, these items make the timing model realistic enough to compare against gem5. The goal is not cycle-exact matching but getting the same **trends** — if gem5 says workload A is 20% faster than workload B, your simulator should agree.

### 3.1 Add branch prediction pipeline latency (1-2 cycles)
`fetch1.rs:174-183`: BTB/TAGE lookup happens in 0 cycles. Add 1-2 cycle latency so the frontend stalls appropriately on back-to-back branches. This primarily affects branch-heavy integer code.

### 3.2 Fix RAS: circular buffer with content checkpointing
`ras.rs`: Pointer-only checkpointing loses stack contents on speculative corruption. Overflow overwrites the same slot instead of wrapping. Rewrite as a circular buffer where snapshots capture the full stack state (or use a copy-on-write scheme for efficiency).

### 3.3 Pipeline I-cache fills instead of bulk stall
`fetch2.rs`: Multi-line I-cache misses sum their penalties into one stall counter. Model the fill pipeline so the first line becomes available while the second is still in-flight.

### 3.4 Make L1 TLBs set-associative (4-way)
Direct-mapped L1 TLBs thrash on common page-stride access patterns. 4-way set-associative with PLRU replacement matches most real cores and gem5 defaults.

### 3.5 Fix PLRU to use tree-based algorithm
`cache/policies/plru.rs:45-71`: Current implementation is a simple bitmask, not tree-based pseudo-LRU. Real hardware and gem5 use a binary tree of decision bits. This matters most at 8-way and 16-way associativity.

### 3.6 Add DRAM timing parameters: tRCD, tWR, tWTR, tCCD
`controller.rs`: These are the most impactful missing parameters:
- **tRCD**: Separate from tRAS — this is the row-activate to column-access delay
- **tWR**: Write recovery time before precharge
- **tWTR**: Write-to-read turnaround penalty
- **tCCD**: Minimum column-to-column command spacing

### 3.7 Add atomic operation latency
`memory2.rs`: LR/SC/AMO use the same path as regular loads/stores. Add 1-3 extra cycles for LR (reservation setup), 2-5 for SC/AMO (cache line locking + RMW). This affects any code using mutexes, spinlocks, or atomic counters.

### 3.8 Increase store drain rate under pressure
`commit.rs:280`: Fixed 1 store/cycle drain rate. Allow 2-4 stores per cycle when the store buffer exceeds a high-water mark (e.g., 75% full). The drain rate should also depend on cache port availability.

### 3.9 Fix PLIC to support >64 interrupt sources
`plic.rs:67-70`: `update_irqs()` takes a `u64` mask. Expand to a bitvector to support the full 1024 sources. Not critical for basic Linux boot (typically <64 devices), but needed for correctness with many VirtIO devices.

> **Checkpoint: Compare against gem5 on SPEC-like benchmarks**
> 1. Configure gem5 with matching parameters: same cache sizes/associativity, same branch predictor, same DRAM timing
> 2. Run a set of benchmarks (Dhrystone, Coremark, simple SPEC INT/FP kernels, or BenchmarkGame programs)
> 3. Compare IPC, cache miss rates, branch misprediction rates, DRAM bandwidth utilization
> 4. Acceptable divergence: **<15% IPC difference** on most workloads, **<5% cache miss rate difference**
> 5. Investigate outliers — they usually point to a specific modeling gap (e.g., store-heavy code → store drain rate, pointer-chasing → TLB modeling)

---

## Phase 4: DRAM Subsystem (DDR4/5 ready)

With timing validated against gem5, build out the DRAM controller for DDR4/5 research.

### 4.1 Rework bank indexing to XOR-based scheme
XOR higher address bits with bank index bits to reduce conflict rate. This is standard in all modern controllers and significantly affects row buffer hit rates.

### 4.2 Add bank group modeling
DDR4: 4 bank groups, tCCD_S (same group) vs tCCD_L (different group). DDR5: 8 bank groups. This is the single biggest DDR4/5 timing differentiator.

### 4.3 Model burst-level timing
DDR4: BL8 (8-beat burst = 4 UI). DDR5: BL16/BL32. Burst length affects column access timing and effective bandwidth for sub-burst transfers.

### 4.4 Add command bus scheduling
Implement FR-FCFS (First-Ready, First-Come First-Served) as the baseline scheduler with open-page and closed-page policies. This is the standard academic baseline and what gem5 uses.

> **Checkpoint: Validate DRAM model against DRAMSim3 or Ramulator2**
> 1. Feed the same memory trace to your DRAM model and DRAMSim3/Ramulator2
> 2. Compare per-bank utilization, row buffer hit rates, average access latency
> 3. Verify that tRCD, tWR, tWTR, tCCD timing constraints are never violated (add assertions)
> 4. Test with STREAM benchmark (bandwidth-bound) and pointer-chasing (latency-bound) to exercise both extremes

---

## Phase 5: Polish for Publication

These items are not critical for correct simulation but are expected in a research-grade tool that others will use and cite.

### 5.1 Fix ROB tag wraparound for long simulations
`rob.rs:165-169`: u32 tag wraps after ~4 billion instructions. MSHR `flush_after()` uses `<=` comparison on raw tag values, which breaks on wraparound. Either widen to u64 or use sequence-aware comparison (epoch-based).

### 5.2 Fix speculative load wakeup race condition
`o3/mod.rs:522-524`: Verify that the issue queue's `select()` checks PRF readiness before issuing a dependent instruction that was woken by a speculative load. If not safe, add a PRF-ready check at select time.

### 5.3 Integrate write-combining buffer in the load path
`write_buffer.rs`: Loads should consult the WCB before going to cache, in case a store was coalesced there but not yet drained.

### 5.4 Free MSHR entries when all waiters are squashed
`cache/mshr.rs:180-192`: After `flush_after()`, entries with zero waiters should be marked available. The cache fill can still install the line, but the MSHR slot should be reusable.

### 5.5 `pc_trace`: replace `Vec::remove(0)` with `VecDeque`
`commit.rs:118`: O(n) per commit → O(1) with VecDeque. Matters for simulation speed.

### 5.6 UART: implement FCR (FIFO control)
`soc/devices/uart.rs:357`: At minimum, handle FIFO enable/reset bits. Full FIFO depth modeling is optional but FCR writes shouldn't be silently dropped.

### 5.7 Add coherence protocol state to cache lines
Add MESI states (even if single-core always uses Modified/Exclusive). This makes the cache line structure ready for future multicore work and is expected in a research simulator's cache model.

### 5.8 Implement DTB auto-generation from SoC config
Replace the static/external DTB with programmatic generation from `SocConfig`. Ensures the DTB always matches the simulated hardware, eliminating a common misconfiguration failure mode.

> **Checkpoint: Validation suite for publication**
> 1. **ISA compliance**: riscv-tests full suite passes (RV64IMAFDC, privileged)
> 2. **Linux boot**: Boot to shell, run `hackbench`, `lmbench`, or a simple multithreaded program (even single-core Linux uses kernel threads)
> 3. **gem5 comparison**: IPC within 10-15% on a benchmark suite, with written explanation for remaining gaps
> 4. **DRAM validation**: Timing matches DRAMSim3/Ramulator2 within 5% on synthetic traces
> 5. **Simulation speed**: Report instructions/second on a reference machine. gem5 does ~200K-1M IPS in detailed mode — matching or beating this is a selling point
> 6. **Regression tests**: Automated CI that runs ISA tests + Linux boot + benchmark IPC comparison on every commit

---

## After all this, is it "done"?

After completing all 5 phases, you will have a simulator that:
- **Correctly implements RV64IMAFDC** with M/S/U privilege modes
- **Boots Linux** with OpenSBI
- **Produces timing results within 10-15% of gem5** for standard workloads
- **Has a realistic DDR4/5 DRAM model** validated against DRAMSim3/Ramulator2
- **Is publishable** with a validation methodology

What would still be missing compared to gem5 for full feature parity:
- **Multicore / cache coherence** (MESI/MOESI protocol, snoop-based or directory-based)
- **Vector extension (RVV)** — increasingly important for RISC-V research
- **Hypervisor extension** — needed for virtualization research
- **Full system emulation** (network devices, GPU passthrough, etc.)
- **Checkpoint/restore** for long-running simulations (SimPoints)
- **Multi-threaded simulation** (gem5 is single-threaded too, so this is a potential advantage)

But none of those are blockers for a first publication or for people to start using it. A **single-core, RV64IMAFDC, Linux-capable simulator with validated timing** is already more useful than most academic simulators, and directly competitive with gem5's RISC-V support (which is itself relatively new and has its own accuracy issues).

---

**(CORRECTED)** The initial audit claimed PTE A/D bits are not maintained during page table walks. This was **incorrect** — `ptw.rs:185-189` correctly calls `update_access_bits()` to set A/D bits with write-back to memory.
