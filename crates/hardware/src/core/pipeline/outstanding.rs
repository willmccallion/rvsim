//! Outstanding-request tracking for the event-driven pipeline.
//!
//! Each in-flight memory operation issued by the pipeline (instruction fetch,
//! demand load, store write-allocate, atomic RMW, page-table walk) is recorded
//! here keyed by its [`ReqId`]. When the matching
//! [`Packet::MemResp`](crate::sim::packet::Packet::MemResp) lands in the
//! pipeline's mailbox, the drain stage looks up the entry, finishes the
//! work the original stage couldn't (apply sign extension, complete the
//! ROB, advance the walk, push a fetch latch entry, …) and forgets it.

use crate::common::{ExceptionStage, PhysAddr, Trap, VirtAddr};
use crate::core::pipeline::latches::ExMem1Entry;
use crate::core::pipeline::rob::RobTag;
use crate::core::units::bru::Ghr;
use crate::core::units::mmu::ptw::WalkState;

/// An instruction-fetch request awaiting its `MemResp`.
///
/// Fetch1 reads instruction bytes synchronously from the RAM fast path —
/// `paddr` is the post-translation physical address used to do that — but
/// issues a [`MemReq`](crate::sim::packet::Packet::MemReq) with `op = Fetch`
/// to the L1I so cache timing flows through the event queue. The outstanding
/// entry holds the prediction snapshot so the drain stage can push a
/// fully-formed [`Fetch1Fetch2Entry`](crate::core::pipeline::latches::Fetch1Fetch2Entry)
/// into the fetch1→fetch2 latch when the response arrives.
#[derive(Clone, Debug)]
pub struct OutstandingFetch {
    /// Monotonically-increasing fetch sequence number assigned at issue
    /// time. The mailbox-drain stage uses this to reorder responses back
    /// into program order before pushing them into the fetch1→fetch2 latch:
    /// a mixed cache-hit/cache-miss burst can return out of order because
    /// the slower path arrives many cycles after the fast one.
    pub fetch_seq: u64,
    /// Program counter being fetched.
    pub pc: u64,
    /// Post-translation address used to issue the `MemReq`.
    pub paddr: PhysAddr,
    /// Whether the branch predictor predicted taken.
    pub pred_taken: bool,
    /// Predicted target address.
    pub pred_target: u64,
    /// Trap surfaced during fetch1 (alignment, deferred page-crossing fault).
    pub trap: Option<Trap>,
    /// Pipeline stage where the trap was first detected.
    pub exception_stage: Option<ExceptionStage>,
    /// Branch-predictor history snapshot captured at fetch time.
    pub ghr_snapshot: Ghr,
    /// Return-address-stack snapshot captured at fetch time.
    pub ras_snapshot: usize,
}

/// A demand load (or atomic / LR) awaiting its `MemResp`.
///
/// The full [`ExMem1Entry`] is kept so the drain stage can replay the load
/// completion logic without recomputing translation, alignment, or
/// control-signal lookups. `paddr` is the translated physical address;
/// `vaddr` is retained for trace + load-queue updates.
#[derive(Clone, Debug)]
pub struct OutstandingLoad {
    /// Original Execute→Memory1 entry, carrying ctrl signals, rd / `rd_phys`,
    /// pc, inst, `fp_flags`, `vec_mem`, `sfence_vma`, and `store_data` (used by AMO
    /// as the second operand).
    pub entry: ExMem1Entry,
    /// Translated physical address.
    pub paddr: PhysAddr,
    /// Pre-translation virtual address (kept for the load queue's address
    /// field and for trace output).
    pub vaddr: VirtAddr,
    /// Deferred PTE A/D bit update from translation (applied at commit).
    pub pte_update: Option<crate::common::PteUpdate>,
}

/// A store awaiting cache write-allocate acknowledgment.
///
/// The store itself resolves its store buffer slot inline at memory1 — this
/// entry tracks the cache-side write-allocate `MemReq` so the LSU's
/// outstanding-count is accurate for back-pressure decisions.
#[derive(Clone, Debug)]
pub struct OutstandingStore {
    /// ROB tag of the store.
    pub rob_tag: RobTag,
    /// Translated physical address of the store.
    pub paddr: PhysAddr,
}

/// A page-table walk in flight.
///
/// `state` holds the live walker state (current level, page-table root PPN,
/// access info). `pte_addr` is the physical address of the PTE the walker
/// is currently waiting on — the drain stage reads its 64-bit value from
/// the RAM fast path before handing it to
/// [`SimState::translate_continue`](crate::sim::state::memory::SimState::translate_continue).
/// `continuation` says what to do once the walk completes.
#[derive(Clone, Debug)]
pub struct OutstandingWalk {
    /// PTW state being advanced.
    pub state: WalkState,
    /// Physical address of the PTE that the outstanding `MemReq` is reading.
    pub pte_addr: PhysAddr,
    /// What to do once the walk completes.
    pub continuation: WalkContinuation,
}

/// What an in-progress walk resumes once it completes.
#[derive(Clone, Debug)]
pub enum WalkContinuation {
    /// An instruction fetch waiting on its translation. The fetch's
    /// `paddr` is filled in when the walk succeeds, then the fetch's
    /// `MemReq` is issued through the L1I.
    Fetch(OutstandingFetch),
    /// A demand load or store waiting on its translation. The
    /// `ExMem1Entry` is re-injected into the Execute→Memory1 latch so
    /// memory1 re-runs with the (now TLB-resident) translation.
    LoadStore(ExMem1Entry),
}
