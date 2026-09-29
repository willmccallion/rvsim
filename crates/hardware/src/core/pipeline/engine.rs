//! Execution engine traits and pipeline type erasure.
//!
//! `ExecutionEngine` is the backend the shared frontend renames into and
//! the memory stages run inside; `PipelineDispatch` is the enum dispatch
//! for type-erased pipeline storage.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::common::InstSeq;
use crate::config::Config;
use crate::core::pipeline::backend::inorder::InOrderEngine;
use crate::core::pipeline::backend::o3::O3Engine;
use crate::core::pipeline::frontend::{Frontend, STAGE_DELAY};
use crate::core::pipeline::latches::{IdExEntry, Latch, RenameIssueEntry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::snapshot::PipelineSnapshot;
use crate::core::pipeline::squash::PendingSquash;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::units::bru::BranchPredictorWrapper;
use crate::sim::components::{CacheId, ComponentId, PipelineId, ReqId};
use crate::sim::packet::Packet;
use crate::sim::topology::{CoreTopology, PrivateCache};
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

/// What renaming one decoded instruction produced.
#[derive(Debug)]
pub enum Renamed {
    /// The instruction has its backend slots; this is what issue sees.
    Accepted(Box<RenameIssueEntry>),
    /// A resource was short; the instruction waits in the frontend.
    Stalled(Box<IdExEntry>),
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

    /// How many instructions the engine can rename this cycle: the least
    /// of its free ROB, buffer and register slots and its rename width.
    fn can_accept(&self) -> usize;

    /// Renames one decoded instruction, allocating its backend slots, and
    /// returns the entry the issue stage works on, or hands the
    /// instruction back when a resource `can_accept` does not cover is
    /// short; the frontend retries it next cycle.
    fn rename(&mut self, state: &mut crate::sim::StageCtx<'_>, id: IdExEntry) -> Renamed;

    /// Flush all speculative state. Committed stores in the store buffer remain.
    fn flush(&mut self, state: &mut crate::sim::CoreCtx<'_>);

    /// Sends the next write of a committed store still buffered, as the
    /// commit stage does each cycle. Returns whether any committed store's
    /// write has yet to be acknowledged.
    fn send_committed_write(&mut self, state: &mut crate::sim::CoreCtx<'_>) -> bool;

    /// The vector configuration an instruction decoded now runs under: the
    /// result of the youngest executed `vsetvl` still in the ROB, else the
    /// architectural CSRs.
    fn vector_config(&self, csrs: &crate::arch::csr::Csrs) -> crate::isa::rvv::VectorConfig {
        self.rob().youngest_vec_csr_update().unwrap_or_else(|| csrs.vector_config())
    }

    /// Whether the backend renames registers, so decode can skip the
    /// intra-bundle RAW hazard check.
    fn has_register_renaming(&self) -> bool;

    /// The reorder buffer.
    fn rob(&self) -> &Rob;

    /// The scalar store buffer.
    fn store_buffer(&self) -> &StoreBuffer;

    /// The scalar store buffer, for memory1 to resolve stores into.
    fn store_buffer_mut(&mut self) -> &mut StoreBuffer;

    /// The vector store buffer younger loads forward from.
    fn vec_store_buffer(&self) -> &crate::core::pipeline::vec_store_buffer::VecStoreBuffer;

    /// The vector store buffer, for the mailbox to acknowledge its writes.
    fn vec_store_buffer_mut(
        &mut self,
    ) -> &mut crate::core::pipeline::vec_store_buffer::VecStoreBuffer;

    /// The load queue, on a backend that tracks loads for ordering
    /// violations.
    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue>;

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
}

/// A trap commit has detected and will squash into once the trap latency
/// has elapsed. Nothing retires in between.
#[derive(Debug, Clone)]
pub struct PendingTrap {
    /// The exception, or the interrupt as it was when detected; an
    /// interrupt is re-evaluated when taken.
    pub trap: crate::isa::privileged::Trap,
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

/// Cycles from commit retiring an instruction to the branch predictor
/// training on it: gem5's `commitToFetchDelay`.
const COMMIT_TO_PREDICTOR_DELAY: u64 = 1;

/// The youngest instruction commit retired in one cycle, sent to the
/// branch predictor.
#[derive(Clone, Copy, Debug)]
pub struct CommitNotice {
    seq: InstSeq,
    sent_at: u64,
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
    /// Memory ops continuing with translations they already hold.
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
    /// The number fetch gives the next instruction it forms.
    pub next_inst_seq: InstSeq,
    /// The youngest instruction commit retired in each recent cycle, on its
    /// way to the branch predictor.
    pub commit_notices: VecDeque<CommitNotice>,
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
            crate::core::pipeline::outstanding::WalkContinuation::LoadStore { entry, .. } => {
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

    /// Numbers the next instruction fetch forms.
    pub const fn alloc_inst_seq(&mut self) -> InstSeq {
        let seq = self.next_inst_seq;
        self.next_inst_seq = seq.next();
        seq
    }

    /// Records `seq` as the youngest instruction commit retired at `now`.
    pub fn note_committed(&mut self, seq: InstSeq, now: u64) {
        self.commit_notices.push_back(CommitNotice { seq, sent_at: now });
    }

    /// Trains the predictor on the commits whose notice has arrived by
    /// `now`. None arrives while a squash is waiting: that squash may still
    /// correct a prediction commit has passed, and gem5's fetch takes a
    /// squash before a commit notice.
    pub fn deliver_commit_notices(&mut self, predictor: &mut BranchPredictorWrapper, now: u64) {
        if self.pending_squash.is_some() {
            return;
        }
        let mut done = None;
        while let Some(notice) = self
            .commit_notices
            .pop_front_if(|notice| notice.sent_at + COMMIT_TO_PREDICTOR_DELAY <= now)
        {
            done = Some(notice.seq);
        }
        if let Some(done) = done {
            predictor.commit(done);
        }
    }

    /// Undoes the predictions `squash` removes. `keep_seq` is the number of
    /// the instruction it keeps, when that is still in the window.
    pub fn squash_predictions(
        &mut self,
        predictor: &mut BranchPredictorWrapper,
        squash: &PendingSquash,
        keep_seq: Option<InstSeq>,
        now: u64,
    ) {
        match (squash.redirect.repair, keep_seq) {
            (Some(repair), _) => repair.apply(predictor),
            (None, Some(keep)) => predictor.squash_after(keep),
            (None, None) => {
                self.deliver_all_commit_notices(predictor);
                predictor.squash_all();
            }
        }
        self.deliver_commit_notices(predictor, now);
    }

    /// Squashes every prediction commit has not retired, training first on
    /// every one it has.
    pub fn flush_predictions(&mut self, predictor: &mut BranchPredictorWrapper) {
        self.deliver_all_commit_notices(predictor);
        predictor.squash_all();
    }

    fn deliver_all_commit_notices(&mut self, predictor: &mut BranchPredictorWrapper) {
        if let Some(notice) = self.commit_notices.back() {
            predictor.commit(notice.seq);
        }
        self.commit_notices.clear();
    }

    /// Files a squash, keeping whichever of it and the pending one takes
    /// precedence (see [`PendingSquash::takes_precedence_over`]).
    pub fn request_squash(&mut self, squash: PendingSquash) {
        let replaces =
            self.pending_squash.is_none_or(|pending| squash.takes_precedence_over(&pending));
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

    /// Drops every fetch in flight: line requests, fetch walks, completed
    /// groups waiting to drain and any fetch hold. Responses for them that
    /// arrive later are discarded rather than entering the new fetch stream.
    pub fn drop_fetches(&mut self) {
        self.outstanding_fetches.clear();
        self.outstanding_walks.retain(|_, walk| {
            !matches!(
                walk.continuation,
                crate::core::pipeline::outstanding::WalkContinuation::Fetch { .. }
            )
        });
        self.fetch_reorder.clear();
        self.fetch_walk_pending = false;
        self.fetch_resume_pc = None;
        self.fetch_hold_until = 0;
        self.next_emit_fetch_seq = self.next_fetch_seq;
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
    pub frontend: Frontend<E>,
    /// Backend execution engine (in-order or out-of-order).
    pub engine: E,
    /// Rename → dispatch latch, consumed by the engine each cycle.
    pub rename_output: Latch<RenameIssueEntry>,
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
    /// Whether a tick this cycle would only count an idle cycle: the hart
    /// waits in WFI with no enabled interrupt pending, and nothing is in
    /// flight in the pipeline, its store buffers or the write-combining
    /// buffer. What the frontend fetched past the WFI may stay: the
    /// frontend does not tick while the hart waits.
    pub fn is_idle(&self, hart: &crate::arch::Hart, units: &crate::core::CoreUnits) -> bool {
        let common = self.engine.common();
        hart.wfi_waiting
            && hart.csrs.mip & hart.csrs.mie == 0
            && self.engine.rob().is_empty()
            && self.engine.store_buffer().is_empty()
            && self.engine.vec_store_buffer().is_empty()
            && !units.wcb.has_pending()
            && matches!(common.trap, TrapProgress::None)
            && common.pending_squash.is_none()
            && common.mailbox.is_empty()
            && common.outstanding_fetches.is_empty()
            && common.outstanding_loads.is_empty()
            && common.outstanding_stores.is_empty()
            && common.outstanding_walks.is_empty()
            && common.forwarded_loads.is_empty()
            && common.commit_notices.is_empty()
            && common.fetch_reorder.is_empty()
            && !common.fetch_walk_pending
            && self.rename_output.is_empty()
            && self.redirect.is_none()
    }

    /// Run one cycle of the entire pipeline.
    ///
    /// Order:
    /// 1. Drain the mailbox — completed loads land in M1→M2; completed walks
    ///    re-inject into Execute→Memory1; completed fetches land in F1→F2.
    /// 2. `engine.tick` — commit, writeback, memory2, memory1, issue, execute.
    /// 3. Frontend — fetch1 / fetch2 / decode / rename.
    pub fn tick(&mut self, state: &mut crate::sim::CoreCtx<'_>) {
        crate::core::pipeline::mailbox::drain(self, &mut state.stage());

        let frontend_empty = self.frontend.is_empty()
            && self.rename_output.is_empty()
            && !self.engine.common().fetch_in_flight();
        self.engine.common_mut().frontend_empty = frontend_empty;
        // The engine dispatches the bundle, or leaves it when a trap or squash
        // makes it wrong-path; the redirect below discards the frontend then.
        let mut dispatch = self.rename_output.take(state.cycle);
        self.engine.tick(state, &mut dispatch, &mut self.redirect);

        if let Some(pc) = self.redirect.take() {
            self.discard_frontend_speculation();
            self.frontend.fetch_pc = pc;
        }

        if state.check_exit().is_none() && !state.hart.wfi_waiting {
            self.frontend.tick(&mut state.stage(), &mut self.engine, &mut self.rename_output);
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
        common.drop_fetches();
        common.vector_config_unresolved = false;
    }

    /// One cycle of draining a flushed pipeline: takes the memory system's
    /// acknowledgements and sends the next committed store's write. Returns
    /// whether any committed store has yet to finish writing.
    pub fn drain_writes(&mut self, state: &mut crate::sim::CoreCtx<'_>) -> bool {
        crate::core::pipeline::mailbox::drain(self, &mut state.stage());
        self.engine.send_committed_write(state)
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
        // Committed stores' writes are still in the memory system; their
        // acknowledgements free store-buffer slots.
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
    /// Builds the configured backend for `core`, fetching from `pc`.
    pub fn new(config: &Config, core: &CoreTopology, pc: u64) -> Self {
        let l1i = core.cache(PrivateCache::L1I);
        let l1d = core.cache(PrivateCache::L1D);
        match config.pipeline.backend {
            BackendType::InOrder => Self::InOrder(Box::new(Pipeline {
                frontend: Frontend::new(pc),
                engine: InOrderEngine::new(config, core.pipeline_id, l1i, l1d),
                rename_output: Latch::new(STAGE_DELAY),
                redirect: None,
            })),
            BackendType::OutOfOrder => Self::OutOfOrder(Box::new(Pipeline {
                frontend: Frontend::new(pc),
                engine: O3Engine::new(config, core.pipeline_id, l1i, l1d),
                rename_output: Latch::new(STAGE_DELAY),
                redirect: None,
            })),
        }
    }

    /// See [`Pipeline::is_idle`].
    pub fn is_idle(&self, hart: &crate::arch::Hart, units: &crate::core::CoreUnits) -> bool {
        match self {
            Self::InOrder(p) => p.is_idle(hart, units),
            Self::OutOfOrder(p) => p.is_idle(hart, units),
        }
    }

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

    /// See [`Pipeline::drain_writes`].
    pub fn drain_writes(&mut self, state: &mut crate::sim::CoreCtx<'_>) -> bool {
        match self {
            Self::InOrder(p) => p.drain_writes(state),
            Self::OutOfOrder(p) => p.drain_writes(state),
        }
    }

    /// Capture a point-in-time snapshot of all inter-stage latch contents.
    pub fn snapshot(&self, width: usize) -> PipelineSnapshot {
        match self {
            Self::InOrder(p) => PipelineSnapshot {
                fetch1_fetch2: p.frontend.fetch1_fetch2.entries().to_vec(),
                fetch2_decode: p.frontend.fetch2_decode.entries().to_vec(),
                decode_rename: p.frontend.decode_rename.entries().to_vec(),
                rename_issue: p.rename_output.entries().to_vec(),
                issue_queue: p.engine.issuer.queue_snapshot(),
                execute_mem1: p.engine.execute_mem1.clone(),
                mem1_mem2: p.engine.mem1_mem2.clone(),
                mem2_wb: p.engine.mem2_wb.clone(),
                width,
            },
            Self::OutOfOrder(p) => PipelineSnapshot {
                fetch1_fetch2: p.frontend.fetch1_fetch2.entries().to_vec(),
                fetch2_decode: p.frontend.fetch2_decode.entries().to_vec(),
                decode_rename: p.frontend.decode_rename.entries().to_vec(),
                rename_issue: p.rename_output.entries().to_vec(),
                issue_queue: p.engine.issue_queue.queue_snapshot(),
                execute_mem1: p.engine.execute_mem1.clone(),
                mem1_mem2: p.engine.mem1_mem2.clone(),
                mem2_wb: p.engine.mem2_wb.clone(),
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

    fn squash(
        keep: u32,
        cause: crate::core::pipeline::squash::SquashCause,
        target: u64,
    ) -> PendingSquash {
        PendingSquash {
            keep_tag: Some(RobTag(keep)),
            redirect: crate::core::pipeline::squash::Redirect::to(target, cause),
            apply_at: 10,
        }
    }

    #[test]
    fn a_commit_reaches_the_predictor_a_cycle_later() {
        let mut predictor =
            crate::core::units::bru::BranchPredictorWrapper::new(&crate::config::Config::default());
        let mut common = BackendCommon::default();
        common.note_committed(InstSeq::new(4), 10);

        common.deliver_commit_notices(&mut predictor, 10);
        let pending_at_commit = common.commit_notices.len();
        common.deliver_commit_notices(&mut predictor, 11);

        assert_eq!((pending_at_commit, common.commit_notices.len()), (1, 0));
    }

    #[test]
    fn a_waiting_squash_holds_commit_notices_back() {
        use crate::core::pipeline::squash::SquashCause;
        let mut predictor =
            crate::core::units::bru::BranchPredictorWrapper::new(&crate::config::Config::default());
        let mut common = BackendCommon::default();
        common.note_committed(InstSeq::new(4), 10);
        common.request_squash(squash(5, SquashCause::Branch, 0x2000));

        common.deliver_commit_notices(&mut predictor, 20);

        assert_eq!(common.commit_notices.len(), 1);
    }

    #[test]
    fn a_mispredict_filed_after_a_violation_behind_it_still_redirects() {
        use crate::core::pipeline::squash::SquashCause;
        let mut common = BackendCommon::default();

        common.request_squash(squash(5, SquashCause::MemoryOrder, 0x1000));
        common.request_squash(squash(5, SquashCause::Branch, 0x2000));

        let taken = common.take_due_squash(10).map(|s| s.redirect.target);
        assert_eq!(taken, Some(0x2000));
    }

    #[test]
    fn a_violation_filed_after_a_mispredict_does_not_replace_it() {
        use crate::core::pipeline::squash::SquashCause;
        let mut common = BackendCommon::default();

        common.request_squash(squash(5, SquashCause::Branch, 0x2000));
        common.request_squash(squash(5, SquashCause::MemoryOrder, 0x1000));

        let taken = common.take_due_squash(10).map(|s| s.redirect.target);
        assert_eq!(taken, Some(0x2000));
    }

    #[test]
    fn test_backend_type_default() {
        assert_eq!(BackendType::default(), BackendType::InOrder);
    }

    #[test]
    fn test_pipeline_dispatch_inorder_tick_flush_snapshot() {
        let config = crate::config::Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let frontend = Frontend::new(state.hart.pc);
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let pipeline = Pipeline {
            frontend,
            engine,
            rename_output: Latch::new(crate::core::pipeline::frontend::STAGE_DELAY),
            redirect: None,
        };
        let mut dispatch = PipelineDispatch::InOrder(Box::new(pipeline));

        dispatch.tick(&mut state);
        dispatch.flush(&mut state);
        let snapshot = dispatch.snapshot(1);
        assert_eq!(snapshot.width, 1);
    }
}
