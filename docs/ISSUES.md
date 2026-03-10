# Microarchitectural Correctness Issues

Findings from a blind code audit of the O3 RISC-V simulator against the RISC-V
privileged specification. The Linux boot failure (kernel paging request oops at
`path_mount` during `devtmpfs_setup`) is a downstream symptom of one or more of
these hardware-level bugs.

---

## FINDING 1 — CRITICAL: Speculative PTE A/D Bit Updates ✅ FIXED

**Location:** `crates/hardware/src/core/units/mmu/ptw.rs:226-231`

**Problem:** The PTW wrote Accessed/Dirty bits directly to the PTE in physical
memory during the page walk. Page walks happen in the Memory1 stage — which is
speculative. If the instruction is later squashed (branch mispredict, memory
ordering violation, or younger-than-faulted instruction), the PTE modification
cannot be rolled back.

**Consequences:**
- Speculatively executed loads set the A bit permanently, corrupting the
  kernel's LRU page reclamation tracking.
- Speculatively executed stores set the D bit on clean pages, causing the
  kernel to skip expected copy-on-write faults or write back pages that were
  never actually dirtied.
- The TLB cached the updated PTE bits, so even after squash the stale A/D
  state persisted in the TLB and was never corrected in RAM.

**Fix applied:**
- PTW computes the updated PTE but does NOT write it to memory.
- A `PteUpdate { pte_addr, pte_value }` is returned via `TranslationResult`.
- The update is threaded through `Mem1Mem2Entry` → `Mem2WbEntry` → `RobEntry`.
- The commit stage writes the PTE to memory only when the instruction retires.
- TLBs now cache the *original* PTE bits (without speculative A/D), so
  subsequent accesses re-walk and generate a fresh `PteUpdate` for commit.

---

## FINDING 2 — CRITICAL: SFENCE.VMA Race Window

**Location:** `crates/hardware/src/core/pipeline/backend/o3/execute.rs:426-427`
(execute-time flush) and `crates/hardware/src/core/pipeline/backend/shared/commit.rs:354-365`
(commit-time re-flush)

**Problem:** SFENCE.VMA flushes TLBs at execute time (line 427) and again at
commit time (lines 362-364). Between these two points, other instructions
already in the Memory1/Memory2 pipeline stages continue executing, performing
page walks that re-insert stale translations into the TLBs.

Additionally, the execute-stage flush does NOT drain the store buffer. If a
preceding store modified a PTE and is still in the store buffer (not yet
committed/drained), the PTW will re-walk the page tables reading the old PTE
from RAM, not the new one in the store buffer.

The simulator acknowledges this bug in a comment at `commit.rs:356-360`.

**Status:** NOT YET FIXED

---

## FINDING 3 — HIGH: Store Buffer Invisible to PTW

**Location:** `crates/hardware/src/core/units/mmu/ptw.rs:200` and store buffer

**Problem:** The PTW reads PTEs directly from the bus (`bus.read_u64(pte_addr)`).
If a store instruction has modified a PTE and the store is still in the store
buffer (state = Ready, not yet drained to memory), the PTW reads the stale PTE
from RAM.

**Timeline:**
1. Kernel writes new PTE via `sd` instruction → data enters store buffer
2. Kernel executes SFENCE.VMA → TLBs flushed
3. Kernel accesses new mapping → TLB miss → PTW walks page table
4. PTW reads PTE from bus → gets old PTE (store not yet drained)
5. Translation fails or returns wrong physical address

This is the root cause underlying Finding 2's comment — the commit-time
re-flush is a band-aid for this deeper issue.

**Status:** NOT YET FIXED

---

## FINDING 4 — HIGH: Speculative MMIO Reads

**Location:** `crates/hardware/src/core/pipeline/backend/shared/memory2.rs:257-274`

**Problem:** When a load's store-buffer forwarding check returns `Miss`, the
load reads directly from the bus. For MMIO addresses (below `cache_base`), this
reads device registers immediately at the Memory2 stage, which is speculative.
MMIO reads have side effects (clearing interrupt status, advancing FIFO
pointers, etc.) that cannot be undone.

The issue-time check (`issue_queue.rs:271-275`) prevents loads from issuing
while older store addresses are unresolved, but this is a best-effort timing
check — it doesn't guarantee correctness at Memory2 time.

**Status:** NOT YET FIXED

---

## FINDING 5 — HIGH: LR/SC Reservation Checked Speculatively

**Location:** `crates/hardware/src/core/pipeline/backend/shared/memory2.rs:108-119`

**Problem:** SC checks and clears the reservation at Memory2 (speculative). If
the SC instruction is later squashed, the reservation has already been cleared,
causing a subsequent (correct) SC to spuriously fail. Similarly, LR sets the
reservation at Memory2 (line 106); if squashed, the reservation persists,
allowing a stale SC to spuriously succeed.

In a single-hart model this is less catastrophic than multi-hart, but it can
cause livelock in LR/SC retry loops — the kernel uses LR/SC for spinlocks and
`cmpxchg`.

**Status:** NOT YET FIXED

---

## FINDING 6 — MEDIUM: User-Mode Force Delegation

**Location:** `crates/hardware/src/core/cpu/trap.rs:134-150`

**Problem:** Any synchronous exception from User mode is force-delegated to
S-mode if STVEC is nonzero, regardless of `medeleg`. Per the RISC-V spec,
undelegated exceptions must trap to M-mode. This could cause the S-mode handler
to receive exceptions it doesn't expect.

**Status:** NOT YET FIXED

---

## FINDING 7 — MEDIUM: D-bit TLB Invalidate Forces Re-walk

**Location:** `crates/hardware/src/core/units/mmu/mod.rs:169-174`

**Problem:** When a store hits a TLB entry with D=0, the simulator invalidates
the L1 DTLB entry and falls through to a full page table walk to set the D bit.
This walk happens in Memory1 (speculative), and combined with Finding 1
(now fixed), the D bit was set speculatively in RAM. This finding is partially
mitigated by the Finding 1 fix (A/D writes are now deferred to commit), but the
performance impact of forcing a full PTW on every first store to a page remains.

**Status:** PARTIALLY MITIGATED (by Finding 1 fix)

---

## Audit Domains That Passed

| Domain | Assessment |
|--------|-----------|
| Precise exceptions | Correct. Faults marked in ROB at execute, taken only at ROB head commit. |
| Pipeline flush completeness | Correct. Full flush scrubs ROB, IQ, SB, LQ, rename map, scoreboard, inter-stage latches, MSHRs. |
| Rename map recovery | Correct. Full flush restores committed map; partial flush rebuilds from surviving ROB entries. |
| Free list reclamation | Correct. All squashed phys regs reclaimed before rebuilding rename map. |
| Trap delegation (medeleg/mideleg) | Correct for S-mode and M-mode traps (User-mode has the force-delegation issue). |
| sepc / mepc | Correct. Points to faulting instruction PC, not PC+4. |
| stval / mtval | Correct. Contains faulting virtual address for page faults. |
| sstatus/mstatus save/restore | Correct. SIE→SPIE, SPP set, SIE cleared on entry; reversed on SRET. |
| MRET/SRET | Correct. Privilege restoration, interrupt enable restoration, MPP/SPP clearing. |
| MPRV | Correct. Data accesses use MPP privilege when MPRV=1; fetches unaffected. |
| SUM | Correct. S-mode U-page access gated; fetch always faults. |
| MXR | Correct. Checked at both TLB lookup and PTW. |
| Store-to-load forwarding | Correct. Newest-first search, proper tag filtering, full/partial overlap handling. |
| Memory ordering violation detection | Correct. Detected when store resolves address, oldest violator flushed. |
| CSR write deferral | Correct. CSR reads speculative, writes deferred to commit, instructions serialized. |
| Interrupt priority | Correct. MEIP > MSIP > MTIP > SEIP > SSIP > STIP. |
