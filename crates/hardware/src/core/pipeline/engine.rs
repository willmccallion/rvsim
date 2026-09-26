//! Execution engine traits and pipeline type erasure.
//!
//! This module defines the trait hierarchy for pluggable backends:
//! 1. **`IssueUnit`** — stage-level trait for instruction issue (FIFO vs O3).
//! 2. **`ExecuteUnit`** — stage-level trait for instruction execution.
//! 3. **`ExecutionEngine`** — high-level trait covering the entire backend.
//! 4. **`PipelineDispatch`** — enum dispatch for type-erased pipeline storage.

use std::collections::{BTreeMap, HashMap};

use crate::core::pipeline::checkpoint::CheckpointTable;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::prf::PhysRegFile;
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::snapshot::PipelineSnapshot;
use crate::core::pipeline::squash::PendingSquash;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::units::vpu::types::VecPhysReg;
use crate::sim::components::{CacheId, ComponentId, PipelineId, ReqId};
use crate::sim::packet::Packet;
use serde::Deserialize;

/// Backend type selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BackendType {
    /// In-order pipeline (default).
    #[default]
    InOrder,
    /// Out-of-order pipeline (future).
    OutOfOrder,
}

/// The execution engine trait — implemented by `InOrderEngine` and `O3Engine`.
///
/// Covers the entire backend: Issue → Execute → Memory1 → Memory2 →
/// Writeback → Commit. The engine owns its in-flight memory bookkeeping
/// (mailbox + outstanding_* + `next_req_id` + cache routing IDs), giving
/// memory1 direct access without splitting `tick` into phases.
pub trait ExecutionEngine {
    /// Run one cycle of all backend stages (reverse order internally).
    ///
    /// The engine sets `redirect` to the PC fetch restarts from when an
    /// instruction squashes the frontend (branch misprediction, trap,
    /// FENCE.I, MRET/SRET).
    fn tick(
        &mut self,
        state: &mut crate::sim::CoreCtx<'_>,
        rename_output: &mut Vec<RenameIssueEntry>,
        redirect: &mut Option<u64>,
    );

    /// How many instructions can the engine accept from rename this cycle?
    fn can_accept(&self) -> usize;

    /// Flush all speculative state. Committed stores in the store buffer remain.
    fn flush(&mut self, state: &mut crate::sim::CoreCtx<'_>);

    /// Write every committed store still buffered to memory.
    fn drain_committed_stores(&mut self, state: &mut crate::sim::CoreCtx<'_>);

    /// The vector configuration an instruction decoded now runs under: the
    /// result of the youngest executed `vsetvl` still in the ROB, else the
    /// architectural CSRs.
    fn vector_config(
        &self,
        state: &crate::sim::CoreCtx<'_>,
    ) -> crate::core::units::vpu::types::VectorConfig {
        self.rob().youngest_vec_csr_update().unwrap_or_else(|| {
            crate::core::units::vpu::types::VectorConfig::from_csrs(&state.hart.csrs)
        })
    }

    /// Access the scoreboard (for rename to mark producers, issue to check readiness).
    fn scoreboard(&self) -> &Scoreboard;
    /// Access the scoreboard mutably (for rename to mark producers).
    fn scoreboard_mut(&mut self) -> &mut Scoreboard;

    /// Access the ROB (for rename to allocate entries, forwarding, etc.).
    fn rob(&self) -> &Rob;
    /// Access the ROB mutably (for rename to allocate entries).
    fn rob_mut(&mut self) -> &mut Rob;

    /// Access the store buffer (for rename to allocate, memory2 for forwarding).
    fn store_buffer(&self) -> &StoreBuffer;
    /// Access the store buffer mutably (for rename to allocate entries).
    fn store_buffer_mut(&mut self) -> &mut StoreBuffer;

    /// The vector store buffer younger loads forward from. `None` for
    /// backends without one.
    fn vec_store_buffer(&self) -> Option<&crate::core::pipeline::vec_store_buffer::VecStoreBuffer> {
        None
    }

    /// Access the vector store buffer mutably (for rename to reserve
    /// entries). `None` for backends without one.
    fn vec_store_buffer_mut(
        &mut self,
    ) -> Option<&mut crate::core::pipeline::vec_store_buffer::VecStoreBuffer> {
        None
    }

    /// Access the speculative rename map (O3 only).
    fn rename_map(&self) -> &RenameMap {
        panic!("rename_map only available for O3 backend")
    }
    /// Access the speculative rename map mutably (O3 only).
    fn rename_map_mut(&mut self) -> &mut RenameMap {
        panic!("rename_map_mut only available for O3 backend")
    }

    /// Access the physical register file (O3 only).
    fn prf(&self) -> &PhysRegFile {
        panic!("prf only available for O3 backend")
    }
    /// Access the physical register file mutably (O3 only).
    fn prf_mut(&mut self) -> &mut PhysRegFile {
        panic!("prf_mut only available for O3 backend")
    }

    /// Access the free list (O3 only).
    fn free_list_mut(&mut self) -> &mut FreeList<PhysReg> {
        panic!("free_list_mut only available for O3 backend")
    }

    /// Access the load queue (O3 only). Returns None for in-order backend.
    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        None
    }

    /// Mutable access to the Execute→Memory1 input latch. The mailbox-drain
    /// stage uses this to re-inject `ExMem1Entry` values after a page-table
    /// walk completes so memory1 reprocesses them with the new TLB entry in
    /// place.
    fn execute_mem1_mut(&mut self) -> &mut Vec<crate::core::pipeline::latches::ExMem1Entry>;

    /// Mutable access to the Memory1→Memory2 latch. Memory1 pushes
    /// completed (non-load or SB-forwarded) entries here directly; the
    /// mailbox-drain stage pushes parked-load completions here once the
    /// matching `MemResp` arrives.
    fn mem1_mem2_mut(&mut self) -> &mut Vec<crate::core::pipeline::latches::Mem1Mem2Entry>;

    /// Shared in-flight memory bookkeeping (mailbox + outstanding tables +
    /// routing IDs).
    fn common(&self) -> &BackendCommon;

    /// Mutable access to the shared bookkeeping. Used by the mailbox-drain
    /// stage on `Pipeline<E>` and by memory1 inside `tick`.
    fn common_mut(&mut self) -> &mut BackendCommon;

    /// Returns true if this backend uses physical register renaming.
    fn has_prf(&self) -> bool {
        false
    }

    /// Returns true if this backend resolves intra-bundle RAW hazards via
    /// register renaming, so decode can skip the bundle-write hazard check.
    /// Default `false` (in-order); O3 overrides.
    fn has_register_renaming(&self) -> bool {
        false
    }

    /// Access the checkpoint table (O3 only).
    fn checkpoint_table(&self) -> &CheckpointTable {
        panic!("checkpoint_table only available for O3 backend")
    }
    /// Access the checkpoint table mutably (O3 only).
    fn checkpoint_table_mut(&mut self) -> &mut CheckpointTable {
        panic!("checkpoint_table_mut only available for O3 backend")
    }
    /// Returns the configured checkpoint count (0 = disabled).
    fn checkpoint_count(&self) -> usize {
        0
    }

    /// Access the vector physical register file (O3 only).
    fn vec_prf(&self) -> &VecPhysRegFile {
        panic!("vec_prf only available for O3 backend")
    }
    /// Access the vector physical register file mutably (O3 only).
    fn vec_prf_mut(&mut self) -> &mut VecPhysRegFile {
        panic!("vec_prf_mut only available for O3 backend")
    }
    /// Access the vector free list mutably (O3 only).
    fn vec_free_list_mut(&mut self) -> &mut FreeList<VecPhysReg> {
        panic!("vec_free_list_mut only available for O3 backend")
    }
}

/// A trap commit has detected and will squash into once the trap latency
/// has elapsed. Nothing retires in between.
#[derive(Debug, Clone)]
pub struct PendingTrap {
    /// The exception, or the interrupt as it was when detected; an
    /// interrupt is re-evaluated when taken.
    pub trap: crate::common::error::Trap,
    /// The PC the trap reports.
    pub epc: u64,
    /// The cycle the pipeline squashes into the handler.
    pub taken_at: u64,
}

/// Where commit is on the way to taking a trap.
#[derive(Debug, Default, Clone)]
pub enum TrapProgress {
    /// No trap detected.
    #[default]
    None,
    /// An enabled interrupt is pending: fetch stops so that everything
    /// already fetched retires before it is taken.
    DrainingForInterrupt,
    /// A detected trap waiting out the trap latency.
    Pending(PendingTrap),
}

impl TrapProgress {
    /// True while fetch stops for an interrupt.
    #[must_use]
    pub const fn stops_fetch(&self) -> bool {
        matches!(self, Self::DrainingForInterrupt)
    }
}

/// State shared by every backend engine: in-flight memory bookkeeping and
/// the routing IDs needed to emit `MemReq` packets and match `MemResp`
/// packets to parked operations.
///
/// Each engine implementation (in-order, out-of-order) embeds a
/// `BackendCommon` and exposes it through
/// [`ExecutionEngine::common`] / [`ExecutionEngine::common_mut`]. The
/// outer [`Pipeline`] uses these accessors to deliver inbound packets to
/// the engine's mailbox and to drain completions, while memory1 inside
/// `engine.tick` mutates the same fields directly to park new requests.
#[derive(Debug, Default)]
pub struct BackendCommon {
    /// Inbound packets the simulator dispatch placed here this cycle.
    pub mailbox: Vec<(ComponentId, Packet)>,
    /// Inflight instruction fetches keyed by request id.
    pub outstanding_fetches: HashMap<ReqId, crate::core::pipeline::outstanding::OutstandingFetch>,
    /// Inflight demand loads (and atomic LR/AMO) keyed by request id.
    pub outstanding_loads: HashMap<ReqId, crate::core::pipeline::outstanding::OutstandingLoad>,
    /// Inflight store write-allocate requests keyed by request id.
    pub outstanding_stores: HashMap<ReqId, crate::core::pipeline::outstanding::OutstandingStore>,
    /// Inflight page-table walks keyed by the current PTE-read request id.
    pub outstanding_walks: HashMap<ReqId, crate::core::pipeline::outstanding::OutstandingWalk>,
    /// Memory ops memory1 could not service yet: loads that partially
    /// overlap an older store still in the store buffer, and LR/AMO ops
    /// with an older store to the same address. They are retried at the
    /// head of every memory1 tick until the blocking store drains (gem5's
    /// `rescheduleMemInst` / `replayMemInst`). Kept out of the
    /// execute→memory1 latch so a blocked op never back-pressures issue:
    /// the store it waits on may sit behind an older, not-yet-issued load.
    pub mem1_replay: Vec<crate::core::pipeline::latches::ExMem1Entry>,
    /// Memory ops waiting out an L2 TLB hit's latency.
    pub mem1_delayed: Vec<crate::core::pipeline::outstanding::DelayedAccess>,
    /// Loads forwarded from the store buffer, waiting out the L1D latency.
    pub forwarded_loads: Vec<crate::core::pipeline::outstanding::ForwardedLoad>,
    /// Secondary request of a line-straddling load, mapped to the request
    /// its [`OutstandingLoad`] is filed under.
    pub load_parts: std::collections::HashMap<ReqId, ReqId>,
    /// Fetch waits until this cycle for an L2 ITLB hit's latency.
    pub fetch_hold_until: u64,
    /// Decode has passed a `vsetvl` that has not executed yet. Younger
    /// instructions are decoded under its result, so decode waits for it.
    pub vector_config_unresolved: bool,
    /// A trap on its way to being taken.
    pub trap: TrapProgress,
    /// The squash execute asked for that has not been taken yet. Commit
    /// retires nothing it will remove.
    pub pending_squash: Option<PendingSquash>,
    /// No instruction is between fetch and rename this cycle, so the ROB
    /// holds everything in flight.
    pub frontend_empty: bool,
    /// Completed but not-yet-emittable fetch groups, keyed by `fetch_seq`.
    /// A group can complete ahead of an older one (a fetch-buffer hit, or
    /// a fetch walk that finishes while the previous line is still
    /// missing); this reorder buffer holds it until every older group has
    /// completed, then drains contiguously into the fetch1→fetch2 latch.
    pub fetch_reorder: BTreeMap<u64, crate::core::pipeline::outstanding::OutstandingFetch>,
    /// Next `fetch_seq` to assign at fetch issue time.
    pub next_fetch_seq: u64,
    /// Next `fetch_seq` we expect to emit to the fetch1→fetch2 latch.
    /// Advanced when a contiguous run drains from `fetch_reorder`; bumped to
    /// `next_fetch_seq` on flush so post-flush fetches stay in order.
    pub next_emit_fetch_seq: u64,
    /// True while a fetch is parked on a page-table walk. fetch1 stalls
    /// instead of re-emitting the same PC every cycle (gem5 `MinorCPU`'s
    /// IFU `ItlbWait` state). Cleared when the matching walk completes.
    pub fetch_walk_pending: bool,
    /// Where fetch continues once a parked fetch walk has completed: the
    /// end of the parked instruction, whose size is known only when its
    /// translation arrives. Taken by the next fetch1 pass.
    pub fetch_resume_pc: Option<u64>,
    /// Oldest load the mailbox drain found inconsistent with an older
    /// load's fresh value (see `LoadQueue::check_coherence_violation`);
    /// the out-of-order engine squashes from it after memory2.
    pub coherence_violation: Option<RobTag>,
    /// Monotonic request-id counter; allocate via [`BackendCommon::alloc_req_id`].
    pub next_req_id: u64,
    /// `PipelineId` of this engine; stamped on every outgoing packet as the
    /// `source` so the response routes back here.
    pub pipeline_id: PipelineId,
    /// L1 instruction cache id (target for fetch requests).
    pub l1_i_id: CacheId,
    /// L1 data cache id (target for load / store / PTE-walk requests).
    pub l1_d_id: CacheId,
}

impl BackendCommon {
    /// Drops every in-flight memory operation younger than `keep_tag`
    /// after a partial squash (branch mispredict or memory-ordering
    /// violation). A squashed load's response must never complete: its
    /// destination physical register may already belong to a newer
    /// instruction. Fetch walks belong to the frontend and are dropped by
    /// the redirect that follows the squash.
    pub fn squash_after(&mut self, keep_tag: RobTag) {
        if self.pending_squash.is_some_and(|s| s.keep_tag.is_none_or(|k| k.is_newer_than(keep_tag)))
        {
            self.pending_squash = None;
        }
        self.outstanding_loads.retain(|_, load| load.entry.rob_tag.is_older_or_eq(keep_tag));
        self.outstanding_walks.retain(|_, walk| match &walk.continuation {
            crate::core::pipeline::outstanding::WalkContinuation::LoadStore(entry) => {
                entry.rob_tag.is_older_or_eq(keep_tag)
            }
            crate::core::pipeline::outstanding::WalkContinuation::Fetch { .. } => true,
        });
        self.mem1_replay.retain(|entry| entry.rob_tag.is_older_or_eq(keep_tag));
        self.mem1_delayed.retain(|access| access.entry.rob_tag.is_older_or_eq(keep_tag));
        self.forwarded_loads.retain(|load| load.entry.rob_tag.is_older_or_eq(keep_tag));
        let loads = &self.outstanding_loads;
        self.load_parts.retain(|_, primary| loads.contains_key(primary));
        if self.coherence_violation.is_some_and(|tag| tag.is_newer_than(keep_tag)) {
            self.coherence_violation = None;
        }
    }

    /// Drops every in-flight memory operation of the backend after a full
    /// squash; fetch walks belong to the frontend and are dropped by the
    /// redirect that follows.
    pub fn squash_all(&mut self) {
        self.pending_squash = None;
        self.outstanding_loads.clear();
        self.outstanding_walks.retain(|_, walk| {
            matches!(
                walk.continuation,
                crate::core::pipeline::outstanding::WalkContinuation::Fetch { .. }
            )
        });
        self.mem1_replay.clear();
        self.mem1_delayed.clear();
        self.forwarded_loads.clear();
        self.load_parts.clear();
        self.coherence_violation = None;
    }

    /// Files a squash, keeping the one that removes more of the window
    /// when one is already pending.
    pub fn request_squash(&mut self, squash: PendingSquash) {
        let replaces = self.pending_squash.is_none_or(|pending| squash.is_older_than(&pending));
        if replaces {
            self.pending_squash = Some(squash);
        }
    }

    /// Takes the pending squash once its latency has elapsed.
    pub fn take_due_squash(&mut self, now: u64) -> Option<PendingSquash> {
        self.pending_squash.take_if(|squash| squash.is_due(now))
    }

    /// True when the pending squash, if any, will remove `tag`.
    #[must_use]
    pub fn will_squash(&self, tag: RobTag) -> bool {
        self.pending_squash.is_some_and(|squash| squash.squashes(tag))
    }

    /// Records a load the drain stage must squash from, keeping the oldest
    /// when several are found in one cycle.
    pub const fn note_coherence_violation(&mut self, tag: RobTag) {
        self.coherence_violation = match self.coherence_violation {
            Some(existing) if existing.is_older_than(tag) => Some(existing),
            _ => Some(tag),
        };
    }

    /// True while an instruction fetch is still waiting on the memory
    /// system: a fetch `MemReq` without its response, a completed fetch
    /// held in the reorder buffer behind an older one, or a fetch parked on
    /// a page-table walk.
    #[must_use]
    pub fn fetch_in_flight(&self) -> bool {
        !self.outstanding_fetches.is_empty()
            || !self.fetch_reorder.is_empty()
            || self.fetch_walk_pending
    }

    /// True while fetch is waiting out an L2 ITLB hit's latency.
    #[must_use]
    pub const fn fetch_held(&self, now: u64) -> bool {
        self.fetch_hold_until > now
    }

    /// Allocates a fresh [`ReqId`] for an outgoing packet. The pipeline
    /// id occupies the top 16 bits so ids are unique across cores; shared
    /// caches and memory controllers key their pending tables by them.
    #[inline]
    pub const fn alloc_req_id(&mut self) -> ReqId {
        let id = self.next_req_id;
        self.next_req_id = id.wrapping_add(1);
        ReqId::new(((self.pipeline_id.val() as u64) << 48) | (id & ((1u64 << 48) - 1)))
    }

    /// Allocates a fresh fetch sequence number for the in-program-order
    /// fetch reorder buffer.
    #[inline]
    pub const fn alloc_fetch_seq(&mut self) -> u64 {
        let seq = self.next_fetch_seq;
        self.next_fetch_seq = seq.wrapping_add(1);
        seq
    }
}

/// The full pipeline combines a frontend and an engine.
///
/// In-flight memory bookkeeping lives on the engine's [`BackendCommon`]; the
/// pipeline reaches it via [`ExecutionEngine::common`] /
/// [`ExecutionEngine::common_mut`].
#[derive(Debug)]
pub struct Pipeline<E: ExecutionEngine> {
    /// Frontend stages: fetch, decode, rename.
    pub frontend: crate::core::pipeline::frontend::Frontend<E>,
    /// Backend execution engine (in-order or out-of-order).
    pub engine: E,
    /// Buffer for rename stage output, consumed by the engine each cycle.
    pub rename_output: Vec<RenameIssueEntry>,
    /// The PC the backend (squash / commit) redirected fetch to this cycle
    /// (branch misprediction, trap, FENCE.I, MRET/SRET); the frontend is
    /// discarded and restarted there after the engine's tick.
    pub redirect: Option<u64>,
}

impl<E: ExecutionEngine> Pipeline<E> {
    /// Places an inbound packet into the engine's mailbox.
    pub fn deliver(&mut self, source: ComponentId, packet: Packet) {
        self.engine.common_mut().mailbox.push((source, packet));
    }
}

impl<E: ExecutionEngine> Pipeline<E> {
    /// Run one cycle of the entire pipeline.
    ///
    /// Order:
    /// 1. Drain the mailbox — completed loads land in M1→M2; completed walks
    ///    re-inject into Execute→Memory1; completed fetches land in F1→F2.
    /// 2. `engine.tick` — commit, writeback, memory2, memory1, issue, execute.
    /// 3. Frontend — fetch1 / fetch2 / decode / rename.
    pub fn tick(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        crate::core::pipeline::mailbox::drain(self, state);

        let frontend_empty = self.frontend.is_empty()
            && self.rename_output.is_empty()
            && !self.engine.common().fetch_in_flight();
        self.engine.common_mut().frontend_empty = frontend_empty;
        self.engine.tick(state, &mut self.rename_output, &mut self.redirect);

        if let Some(pc) = self.redirect.take() {
            self.discard_frontend_speculation();
            self.frontend.fetch_pc = pc;
        }

        if state.check_exit().is_none() && !state.hart.wfi_waiting {
            self.frontend.tick(state, &mut self.engine, &mut self.rename_output);
        }
    }

    /// Drops wrong-path frontend speculation: the fetch latches, in-flight
    /// fetches and fetch walks, and the fetch reorder buffer. Backend
    /// resources (outstanding loads, stores and their walks) belong to ROB
    /// entries a redirect leaves intact, so they survive.
    fn discard_frontend_speculation(&mut self) {
        self.frontend.flush();
        self.rename_output.clear();
        let common = self.engine.common_mut();
        common.outstanding_fetches.clear();
        common.outstanding_walks.retain(|_, walk| {
            !matches!(
                walk.continuation,
                crate::core::pipeline::outstanding::WalkContinuation::Fetch { .. }
            )
        });
        common.fetch_reorder.clear();
        common.fetch_walk_pending = false;
        common.fetch_resume_pc = None;
        common.fetch_hold_until = 0;
        common.vector_config_unresolved = false;
        // Straggler responses for pre-flush fetches are dropped rather than
        // entering the post-flush fetch stream.
        common.next_emit_fetch_seq = common.next_fetch_seq;
    }

    /// Flush the pipeline and write its committed stores to memory; fetch
    /// restarts at the hart's architectural PC.
    pub fn drain(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        self.flush(state);
        self.engine.drain_committed_stores(state);
    }

    /// Flush the entire pipeline; fetch restarts at the hart's
    /// architectural PC.
    pub fn flush(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        self.frontend.fetch_pc = state.hart.pc;
        self.redirect = None;
        self.discard_frontend_speculation();
        let common = self.engine.common_mut();
        common.mailbox.clear();
        common.outstanding_loads.clear();
        common.outstanding_stores.clear();
        common.outstanding_walks.clear();
        common.load_parts.clear();
        common.mem1_replay.clear();
        common.mem1_delayed.clear();
        common.forwarded_loads.clear();
        common.coherence_violation = None;
        common.pending_squash = None;
        common.trap = TrapProgress::None;
        self.engine.flush(state);
    }
}

/// Type-erased pipeline stored per core on the simulator.
#[derive(Debug)]
pub enum PipelineDispatch {
    /// In-order pipeline.
    InOrder(Box<Pipeline<crate::core::pipeline::backend::inorder::InOrderEngine>>),
    /// Out-of-order pipeline.
    OutOfOrder(Box<Pipeline<crate::core::pipeline::backend::o3::O3Engine>>),
}

impl PipelineDispatch {
    /// Run one cycle.
    pub fn tick(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        match self {
            Self::InOrder(p) => p.tick(state),
            Self::OutOfOrder(p) => p.tick(state),
        }
    }

    /// Places an inbound packet into the pipeline's mailbox.
    pub fn deliver(&mut self, source: ComponentId, packet: Packet) {
        match self {
            Self::InOrder(p) => p.deliver(source, packet),
            Self::OutOfOrder(p) => p.deliver(source, packet),
        }
    }

    /// The PC fetch continues from.
    #[must_use]
    pub const fn fetch_pc(&self) -> u64 {
        match self {
            Self::InOrder(p) => p.frontend.fetch_pc,
            Self::OutOfOrder(p) => p.frontend.fetch_pc,
        }
    }

    /// Points fetch at `pc` without disturbing anything in flight; for
    /// initialisation, after the hart's architectural PC has been set.
    pub const fn restart_fetch_at(&mut self, pc: u64) {
        match self {
            Self::InOrder(p) => p.frontend.fetch_pc = pc,
            Self::OutOfOrder(p) => p.frontend.fetch_pc = pc,
        }
    }

    /// Flush.
    pub fn flush(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        match self {
            Self::InOrder(p) => p.flush(state),
            Self::OutOfOrder(p) => p.flush(state),
        }
    }

    /// See [`Pipeline::drain`].
    pub fn drain(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        match self {
            Self::InOrder(p) => p.drain(state),
            Self::OutOfOrder(p) => p.drain(state),
        }
    }

    /// Capture a point-in-time snapshot of all inter-stage latch contents.
    pub fn snapshot(&self, width: usize) -> PipelineSnapshot {
        match self {
            Self::InOrder(p) => PipelineSnapshot {
                fetch1_fetch2: p.frontend.fetch1_fetch2.clone(),
                fetch2_decode: p.frontend.fetch2_decode.clone(),
                decode_rename: p.frontend.decode_rename.clone(),
                rename_issue: p.rename_output.clone(),
                issue_queue: p.engine.issuer.queue_snapshot(),
                execute_mem1: p.engine.execute_mem1.clone(),
                mem1_mem2: p.engine.mem1_mem2.clone(),
                mem2_wb: p.engine.mem2_wb.clone(),
                fetch1_stall: p.frontend.fetch1_stall,
                fetch2_stall: p.frontend.fetch2_stall,
                mem1_stall: 0,
                width,
            },
            Self::OutOfOrder(p) => PipelineSnapshot {
                fetch1_fetch2: p.frontend.fetch1_fetch2.clone(),
                fetch2_decode: p.frontend.fetch2_decode.clone(),
                decode_rename: p.frontend.decode_rename.clone(),
                rename_issue: p.rename_output.clone(),
                issue_queue: p.engine.issue_queue.queue_snapshot(),
                execute_mem1: p.engine.execute_mem1.clone(),
                mem1_mem2: p.engine.mem1_mem2.clone(),
                mem2_wb: p.engine.mem2_wb.clone(),
                fetch1_stall: p.frontend.fetch1_stall,
                fetch2_stall: p.frontend.fetch2_stall,
                mem1_stall: 0, // O3 uses per-entry complete_cycle, no global stall
                width,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pipeline::backend::inorder::InOrderEngine;
    use crate::core::pipeline::frontend::Frontend;

    #[test]
    fn test_backend_type_default() {
        assert_eq!(BackendType::default(), BackendType::InOrder);
    }

    #[test]
    fn test_pipeline_dispatch_inorder_tick_flush_snapshot() {
        let config = crate::config::Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let frontend = Frontend::new(config.pipeline.width, state.hart.pc);
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let pipeline = Pipeline { frontend, engine, rename_output: Vec::new(), redirect: None };
        let mut dispatch = PipelineDispatch::InOrder(Box::new(pipeline));

        dispatch.tick(&mut state);
        dispatch.flush(&mut state);
        let snapshot = dispatch.snapshot(1);
        assert_eq!(snapshot.width, 1);
    }
}
