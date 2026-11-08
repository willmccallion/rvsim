//! Execution engine traits and pipeline type erasure.
//!
//! `ExecutionEngine` is the backend the shared frontend renames into and
//! the memory stages run inside; `PipelineDispatch` is the enum dispatch
//! for type-erased pipeline storage.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::arch::Hart;
use crate::arch::csr::Csrs;
use crate::common::{FetchSeq, InstSeq, VirtAddr};
use crate::config::{BackendKind, Config};
use crate::isa::privileged::Trap;
use crate::isa::rvv::VectorConfig;
use crate::sim::components::{CacheId, ComponentId, PipelineId, ReqId};
use crate::sim::packet::Packet;
use crate::soc::topology::{CoreTopology, PrivateCache};
use crate::uarch::CoreUnits;
use crate::uarch::bpred::BranchPredictor;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::inorder::InOrderEngine;
use crate::uarch::pipeline::backend::o3::O3Engine;
use crate::uarch::pipeline::frontend::Frontend;
use crate::uarch::pipeline::latches::ExMem1Entry;
use crate::uarch::pipeline::latches::Mem1Mem2Entry;
use crate::uarch::pipeline::latches::Mem2WbEntry;
use crate::uarch::pipeline::latches::{
    IdExEntry, IssueRegisters, Latch, RenameIssueEntry, SourceReadiness,
};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::mailbox::drain;
use crate::uarch::pipeline::outstanding::DelayedAccess;
use crate::uarch::pipeline::outstanding::ForwardedLoad;
use crate::uarch::pipeline::outstanding::OutstandingFetch;
use crate::uarch::pipeline::outstanding::OutstandingLoad;
use crate::uarch::pipeline::outstanding::OutstandingStore;
use crate::uarch::pipeline::outstanding::OutstandingWalk;
use crate::uarch::pipeline::outstanding::WalkContinuation;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, Rob, RobTag};
use crate::uarch::pipeline::snapshot::LatchSnapshot;
use crate::uarch::pipeline::squash::PendingSquash;
use crate::uarch::pipeline::squash::SquashScope;
use crate::uarch::pipeline::trace::Departure;

/// Where the next fetch1 pass starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FetchRestart {
    /// At the frontend's fetch PC.
    #[default]
    FromPc,
    /// Nowhere yet: a fetch is parked on a page-table walk, and fetch1
    /// waits for it rather than emit the same PC every cycle (gem5
    /// `MinorCPU`'s IFU `ItlbWait` state).
    AwaitingWalk,
    /// At the instruction a completed walk or an L2 ITLB hit held fetch
    /// on, fetched again now its translation is in the L1 TLB.
    At(VirtAddr),
}

/// What renaming one decoded instruction produced.
#[derive(Debug)]
pub enum Renamed<R> {
    /// The instruction has its backend slots; this is what issue sees.
    Accepted(Box<RenameIssueEntry<R>>),
    /// A resource was short; the instruction waits in the frontend.
    Stalled(Box<IdExEntry>),
}

/// What decode does with an instruction that reads a register an earlier
/// one in its decode bundle writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleHazard {
    /// Nothing: rename maps the read to the earlier write's register.
    Renamed,
    /// It ends the bundle before the reader, which decodes once the writer
    /// has issued.
    EndsBundle,
}

/// The execution engine trait — implemented by `InOrderEngine` and `O3Engine`.
///
/// Covers the entire backend: Issue → Execute → Memory1 → Memory2 →
/// Writeback → Commit. The engine owns its in-flight memory bookkeeping
/// (mailbox + outstanding_* + `next_req_id` + cache routing IDs), giving
/// memory1 direct access without splitting `tick` into phases.
pub trait ExecutionEngine {
    /// The registers this backend tracks an issuing instruction's operands by.
    type Regs: IssueRegisters;

    /// Run one cycle of all backend stages (reverse order internally).
    ///
    /// The engine sets `redirect` to the PC fetch restarts from when an
    /// instruction squashes the frontend (branch misprediction, trap,
    /// FENCE.I, MRET/SRET).
    fn tick(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<RenameIssueEntry<Self::Regs>>,
        redirect: &mut Option<VirtAddr>,
    );

    /// How many instructions the engine can rename this cycle: the least
    /// of its free ROB, buffer and register slots and its rename width.
    fn can_accept(&self) -> usize;

    /// Renames one decoded instruction, allocating its backend slots, and
    /// returns the entry the issue stage works on, or hands the
    /// instruction back when a resource `can_accept` does not cover is
    /// short; the frontend retries it next cycle.
    fn rename(&mut self, state: &mut StageCtx<'_>, decoded: IdExEntry) -> Renamed<Self::Regs>;

    /// Flush all speculative state, in-flight loads and data walks included.
    /// Committed stores in the store buffer remain.
    fn flush(&mut self, state: &mut CoreCtx<'_>);

    /// Sends the next write of a committed store still buffered, as the
    /// commit stage does each cycle. Returns whether any committed store's
    /// write has yet to be acknowledged.
    fn send_committed_write(&mut self, state: &mut CoreCtx<'_>) -> bool;

    /// The vector configuration an instruction decoded now runs under: the
    /// result of the youngest executed `vsetvl` still in the ROB, else the
    /// architectural CSRs.
    fn vector_config(&self, csrs: &Csrs) -> VectorConfig {
        self.rob().youngest_vec_csr_update().unwrap_or_else(|| csrs.vector_config())
    }

    /// What decode does with an instruction that reads a register an
    /// earlier one in its bundle writes.
    fn bundle_hazard(&self) -> BundleHazard;

    /// Whether fetch spends the cycle this tick's redirect reaches it
    /// squashing and fetches from the target the cycle after: every
    /// redirect on the out-of-order engine, as gem5's O3 fetch does, and a
    /// redirect taken at commit on the in-order one, whose execute-time
    /// redirects already wait `redirect_latency`.
    fn fetch_squashes_for_a_cycle(&self) -> bool;

    /// Whether a taken squash still holds rename, so the cycle is not idle.
    fn is_recovering_from_squash(&self) -> bool;

    /// The reorder buffer.
    fn rob(&self) -> &Rob;

    /// The scalar store buffer.
    fn store_buffer(&self) -> &StoreBuffer;

    /// The scalar store buffer, for memory1 to resolve stores into.
    fn store_buffer_mut(&mut self) -> &mut StoreBuffer;

    /// The vector store buffer younger loads forward from.
    fn vec_store_buffer(&self) -> &VecStoreBuffer;

    /// The vector store buffer, for the mailbox to acknowledge its writes.
    fn vec_store_buffer_mut(&mut self) -> &mut VecStoreBuffer;

    /// The load queue, on a backend that tracks loads for ordering
    /// violations.
    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue>;

    /// Mutable access to the Execute→Memory1 input latch. The mailbox-drain
    /// stage uses this to re-inject `ExMem1Entry` values after a page-table
    /// walk completes so memory1 reprocesses them with the new TLB entry in
    /// place.
    fn execute_mem1_mut(&mut self) -> &mut Vec<ExMem1Entry>;

    /// Mutable access to the Memory1→Memory2 latch. Memory1 pushes
    /// completed (non-load or SB-forwarded) entries here directly; the
    /// mailbox-drain stage pushes parked-load completions here once the
    /// matching `MemResp` arrives.
    fn mem1_mem2_mut(&mut self) -> &mut Vec<Mem1Mem2Entry>;

    /// The issue queue's entries, oldest first.
    fn issue_snapshot(&self) -> Vec<RenameIssueEntry<SourceReadiness>>;

    /// The Execute→Memory1 latch.
    fn execute_mem1(&self) -> &[ExMem1Entry];

    /// The Memory1→Memory2 latch.
    fn mem1_mem2(&self) -> &[Mem1Mem2Entry];

    /// The Memory2→Writeback latch.
    fn mem2_wb(&self) -> &[Mem2WbEntry];

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
    pub trap: Trap,
    /// The PC the trap reports.
    pub epc: VirtAddr,
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

/// The L1D flush a FENCE.I at the ROB head needs before it retires, when
/// no cache below keeps instruction fetches coherent with the L1D.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataCacheFlush {
    /// No flush asked for.
    #[default]
    Idle,
    /// Asked for under this correlator; not yet acknowledged.
    Requested(ReqId),
    /// Acknowledged: the FENCE.I may retire.
    Done,
}

/// Instruction fetches in flight: line requests, groups waiting to drain
/// in order, and where fetch restarts after a walk or an L2 ITLB hit.
#[derive(Debug, Default)]
pub struct FetchInFlight {
    /// Inflight instruction fetches keyed by request id.
    pub outstanding_fetches: HashMap<ReqId, OutstandingFetch>,
    /// Fetch waits until this cycle for an L2 ITLB hit's latency.
    pub fetch_hold_until: u64,
    /// Completed but not-yet-emittable fetch groups, keyed by `fetch_seq`.
    /// A group can complete ahead of an older one (a fetch-buffer hit, or
    /// a fetch walk that finishes while the previous line is still
    /// missing); this reorder buffer holds it until every older group has
    /// completed, then drains contiguously into the fetch1→fetch2 latch.
    pub fetch_reorder: BTreeMap<FetchSeq, OutstandingFetch>,
    /// Next `fetch_seq` to assign at fetch issue time.
    pub next_fetch_seq: FetchSeq,
    /// Next `fetch_seq` we expect to emit to the fetch1→fetch2 latch.
    /// Advanced when a contiguous run drains from `fetch_reorder`; bumped to
    /// `next_fetch_seq` on flush so post-flush fetches stay in order.
    pub next_emit_fetch_seq: FetchSeq,
    /// Where the next fetch1 pass starts.
    pub fetch_restart: FetchRestart,
}

/// Data accesses in flight: requests the caches have not answered, and
/// memory ops waiting in memory1 or on store-buffer forwarding.
#[derive(Debug, Default)]
pub struct MemoryInFlight {
    /// Inflight demand loads (and atomic LR/AMO) keyed by request id.
    pub outstanding_loads: HashMap<ReqId, OutstandingLoad>,
    /// Inflight store write-allocate requests keyed by request id.
    pub outstanding_stores: HashMap<ReqId, OutstandingStore>,
    /// Inflight page-table walks keyed by the current PTE-read request id.
    pub outstanding_walks: HashMap<ReqId, OutstandingWalk>,
    /// Memory ops memory1 could not service yet: loads that partially
    /// overlap an older store still in the store buffer, and LR/AMO ops
    /// with an older store to the same address. They are retried at the
    /// head of every memory1 tick until the blocking store drains (gem5's
    /// `rescheduleMemInst` / `replayMemInst`). Kept out of the
    /// execute→memory1 latch so a blocked op never back-pressures issue:
    /// the store it waits on may sit behind an older, not-yet-issued load.
    pub mem1_replay: Vec<ExMem1Entry>,
    /// Memory ops continuing with translations they already hold.
    pub mem1_delayed: Vec<DelayedAccess>,
    /// Loads forwarded from the store buffer, waiting out the forwarding
    /// latency.
    pub forwarded_loads: Vec<ForwardedLoad>,
    /// Loads forwarded this cycle with no latency, finalized by memory1 and
    /// written back once it has run.
    pub forwarded_results: Vec<Mem2WbEntry>,
    /// Secondary request of a line-straddling load, mapped to the request
    /// its [`OutstandingLoad`](crate::uarch::pipeline::outstanding::OutstandingLoad) is filed under.
    pub load_parts: std::collections::HashMap<ReqId, ReqId>,
}

impl FetchInFlight {
    /// True while an instruction fetch is still waiting on the memory
    /// system: a fetch `MemReq` without its response, a completed fetch
    /// held in the reorder buffer behind an older one, or a fetch parked on
    /// a page-table walk.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        !self.outstanding_fetches.is_empty()
            || !self.fetch_reorder.is_empty()
            || self.fetch_restart == FetchRestart::AwaitingWalk
    }
}

impl MemoryInFlight {
    /// True while a data request awaits the caches or a forwarded load
    /// awaits writeback.
    #[must_use]
    pub fn is_waiting(&self) -> bool {
        !self.outstanding_loads.is_empty()
            || !self.outstanding_stores.is_empty()
            || !self.outstanding_walks.is_empty()
            || !self.forwarded_loads.is_empty()
            || !self.forwarded_results.is_empty()
    }
}

/// What commit is in the middle of: a trap on its way, a squash execute
/// asked for, an L1D flush, and the retirements the predictor has not
/// heard of yet.
#[derive(Debug, Default)]
pub struct CommitControl {
    /// A trap on its way to being taken.
    pub trap: TrapProgress,
    /// Where the L1D flush the FENCE.I at the ROB head waits for stands.
    pub data_cache_flush: DataCacheFlush,
    /// The ROB head as this cycle began, before its commit.
    pub head_at_cycle_start: HeadAtCycleStart,
    /// The squash execute asked for that has not been taken yet. Commit
    /// retires nothing it will remove.
    pub pending_squash: Option<PendingSquash>,
    /// The youngest instruction commit retired in each recent cycle, on its
    /// way to the branch predictor.
    pub commit_notices: VecDeque<CommitNotice>,
    /// Oldest load the mailbox drain found inconsistent with an older
    /// load's fresh value (see `LoadQueue::check_coherence_violation`);
    /// the out-of-order engine squashes from it after memory2.
    pub coherence_violation: Option<RobTag>,
}

impl CommitControl {
    /// True while commit has a trap to take, a squash to apply or
    /// retirements to tell the predictor of.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        !matches!(self.trap, TrapProgress::None)
            || self.pending_squash.is_some()
            || !self.commit_notices.is_empty()
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
    /// Instruction fetches in flight.
    pub fetch: FetchInFlight,
    /// Data accesses in flight between the memory stages and the caches.
    pub memory: MemoryInFlight,
    /// What commit is in the middle of: a trap, a squash, a cache flush.
    pub commit: CommitControl,
    /// Inbound packets the simulator dispatch placed here this cycle.
    pub mailbox: Vec<(ComponentId, Packet)>,
    /// Decode has passed a `vsetvl` that has not executed yet. Younger
    /// instructions are decoded under its result, so decode waits for it.
    pub vector_config_unresolved: bool,
    /// The number fetch gives the next instruction it forms.
    pub next_inst_seq: InstSeq,
    /// No instruction is between fetch and rename this cycle, so the ROB
    /// holds everything in flight.
    pub frontend_empty: bool,
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
    /// Drops every in-flight memory operation `scope` squashes.
    pub fn squash(&mut self, scope: SquashScope) {
        match scope {
            SquashScope::After(keep) => self.squash_after(keep),
            SquashScope::All => self.squash_all(),
        }
    }

    /// Drops every in-flight memory operation younger than `keep_tag`
    /// after a partial squash (branch mispredict or memory-ordering
    /// violation). A squashed load's response must never complete: its
    /// destination physical register may already belong to a newer
    /// instruction. Fetch walks belong to the frontend and are dropped by
    /// the redirect that follows the squash.
    pub fn squash_after(&mut self, keep_tag: RobTag) {
        if self
            .commit
            .pending_squash
            .is_some_and(|s| s.keep_tag.is_none_or(|k| k.is_newer_than(keep_tag)))
        {
            self.commit.pending_squash = None;
        }
        self.memory
            .outstanding_loads
            .retain(|_, load| load.entry.op.rob_tag.is_older_or_eq(keep_tag));
        self.memory.outstanding_walks.retain(|_, walk| match &walk.continuation {
            WalkContinuation::LoadStore { entry, .. } => entry.op.rob_tag.is_older_or_eq(keep_tag),
            WalkContinuation::Fetch { .. } => true,
        });
        self.memory.mem1_replay.retain(|entry| entry.op.rob_tag.is_older_or_eq(keep_tag));
        self.memory.mem1_delayed.retain(|access| access.entry.op.rob_tag.is_older_or_eq(keep_tag));
        self.memory.forwarded_loads.retain(|load| load.entry.op.rob_tag.is_older_or_eq(keep_tag));
        self.memory.forwarded_results.retain(|load| load.op.rob_tag.is_older_or_eq(keep_tag));
        let loads = &self.memory.outstanding_loads;
        self.memory.load_parts.retain(|_, primary| loads.contains_key(primary));
        if self.commit.coherence_violation.is_some_and(|tag| tag.is_newer_than(keep_tag)) {
            self.commit.coherence_violation = None;
        }
    }

    /// Drops every in-flight memory operation of the backend after a full
    /// squash; fetch walks belong to the frontend and are dropped by the
    /// redirect that follows.
    pub fn squash_all(&mut self) {
        self.commit.pending_squash = None;
        self.memory.outstanding_loads.clear();
        self.memory
            .outstanding_walks
            .retain(|_, walk| matches!(walk.continuation, WalkContinuation::Fetch { .. }));
        self.memory.mem1_replay.clear();
        self.memory.mem1_delayed.clear();
        self.memory.forwarded_loads.clear();
        self.memory.forwarded_results.clear();
        self.memory.load_parts.clear();
        self.commit.coherence_violation = None;
    }

    /// Numbers the next instruction fetch forms.
    pub const fn alloc_inst_seq(&mut self) -> InstSeq {
        let seq = self.next_inst_seq;
        self.next_inst_seq = seq.next();
        seq
    }

    /// Records `seq` as the youngest instruction commit retired at `now`.
    pub fn note_committed(&mut self, seq: InstSeq, now: u64) {
        self.commit.commit_notices.push_back(CommitNotice { seq, sent_at: now });
    }

    /// Trains the predictor on the commits whose notice has arrived by
    /// `now`. None arrives while a squash is waiting: that squash may still
    /// correct a prediction commit has passed, and gem5's fetch takes a
    /// squash before a commit notice.
    pub fn deliver_commit_notices(&mut self, predictor: &mut BranchPredictor, now: u64) {
        if self.commit.pending_squash.is_some() {
            return;
        }
        let mut done = None;
        while let Some(notice) = self
            .commit
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
        predictor: &mut BranchPredictor,
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
    pub fn flush_predictions(&mut self, predictor: &mut BranchPredictor) {
        self.deliver_all_commit_notices(predictor);
        predictor.squash_all();
    }

    fn deliver_all_commit_notices(&mut self, predictor: &mut BranchPredictor) {
        if let Some(notice) = self.commit.commit_notices.back() {
            predictor.commit(notice.seq);
        }
        self.commit.commit_notices.clear();
    }

    /// Files a squash, keeping whichever of it and the pending one takes
    /// precedence (see [`PendingSquash::takes_precedence_over`]).
    pub fn request_squash(&mut self, squash: PendingSquash) {
        let replaces =
            self.commit.pending_squash.is_none_or(|pending| squash.takes_precedence_over(&pending));
        if replaces {
            self.commit.pending_squash = Some(squash);
        }
    }

    /// Takes the pending squash once its latency has elapsed.
    pub fn take_due_squash(&mut self, now: u64) -> Option<PendingSquash> {
        self.commit.pending_squash.take_if(|squash| squash.is_due(now))
    }

    /// True when the pending squash, if any, will remove `tag`.
    #[must_use]
    pub fn will_squash(&self, tag: RobTag) -> bool {
        self.commit.pending_squash.is_some_and(|squash| squash.squashes(tag))
    }

    /// Records a load the drain stage must squash from, keeping the oldest
    /// when several are found in one cycle.
    pub const fn note_coherence_violation(&mut self, tag: RobTag) {
        self.commit.coherence_violation = match self.commit.coherence_violation {
            Some(existing) if existing.is_older_than(tag) => Some(existing),
            _ => Some(tag),
        };
    }

    /// Drops every fetch in flight: line requests, fetch walks, completed
    /// groups waiting to drain and any fetch hold. Responses for them that
    /// arrive later are discarded rather than entering the new fetch stream.
    /// The instructions fetch has numbered but not yet handed to the
    /// fetch1→fetch2 latch: those in groups waiting on their line.
    pub fn in_flight_fetch_seqs(&self) -> impl Iterator<Item = InstSeq> + '_ {
        self.fetch
            .fetch_reorder
            .values()
            .flat_map(|group| group.entries.iter())
            .map(|entry| entry.seq)
    }

    pub fn drop_fetches(&mut self) {
        self.fetch.outstanding_fetches.clear();
        self.memory
            .outstanding_walks
            .retain(|_, walk| !matches!(walk.continuation, WalkContinuation::Fetch { .. }));
        self.fetch.fetch_reorder.clear();
        self.fetch.fetch_restart = FetchRestart::FromPc;
        self.fetch.fetch_hold_until = 0;
        self.fetch.next_emit_fetch_seq = self.fetch.next_fetch_seq;
    }

    /// True while fetch is waiting out an L2 ITLB hit's latency.
    #[must_use]
    pub const fn fetch_held(&self, now: u64) -> bool {
        self.fetch.fetch_hold_until > now
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
    pub const fn alloc_fetch_seq(&mut self) -> FetchSeq {
        let seq = self.fetch.next_fetch_seq;
        self.fetch.next_fetch_seq = seq.next();
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
    pub rename_output: Latch<RenameIssueEntry<E::Regs>>,
    /// The PC the backend (squash / commit) redirected fetch to this cycle
    /// (branch misprediction, trap, FENCE.I, MRET/SRET); the frontend is
    /// discarded and restarted there after the engine's tick.
    pub redirect: Option<VirtAddr>,
    /// Set by a backend redirect until rename hands on the first bundle
    /// from the new path; each cycle in between is a control stall.
    recovering_from_redirect: bool,
}

impl<E: ExecutionEngine> Pipeline<E> {
    /// A pipeline around `engine`, fetching from `pc`.
    fn new(engine: E, config: &Config, pc: VirtAddr) -> Self {
        Self {
            frontend: Frontend::new(pc, &config.pipeline),
            engine,
            rename_output: Latch::new(config.pipeline.rename_issue_latency - 1),
            redirect: None,
            recovering_from_redirect: false,
        }
    }

    /// A point-in-time copy of every inter-stage latch.
    fn snapshot(&self, width: usize) -> LatchSnapshot {
        LatchSnapshot {
            fetch1_fetch2: self.frontend.fetch1_fetch2.entries().cloned().collect(),
            fetch2_decode: self.frontend.fetch2_decode.entries().cloned().collect(),
            decode_rename: self.frontend.decode_rename.entries().cloned().collect(),
            rename_issue: self.rename_output.entries().map(RenameIssueEntry::snapshot).collect(),
            issue_queue: self.engine.issue_snapshot(),
            execute_mem1: self.engine.execute_mem1().to_vec(),
            mem1_mem2: self.engine.mem1_mem2().to_vec(),
            mem2_wb: self.engine.mem2_wb().to_vec(),
            width,
        }
    }

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
    pub fn is_idle(&self, hart: &Hart, units: &CoreUnits) -> bool {
        let common = self.engine.common();
        hart.wfi_waiting
            && hart.csrs.mip & hart.csrs.mie == 0
            && self.engine.rob().is_empty()
            && self.engine.store_buffer().is_empty()
            && self.engine.vec_store_buffer().is_empty()
            && !units.wcb.has_pending()
            && !common.commit.is_busy()
            && common.mailbox.is_empty()
            && !common.fetch.is_waiting()
            && !common.memory.is_waiting()
            && self.rename_output.is_empty()
            && self.redirect.is_none()
            && !self.engine.is_recovering_from_squash()
    }

    /// Run one cycle of the entire pipeline.
    ///
    /// Order:
    /// 1. Drain the mailbox — completed loads land in M1→M2; completed walks
    ///    re-inject into Execute→Memory1; completed fetches land in F1→F2.
    /// 2. `engine.tick` — commit, writeback, memory2, memory1, issue, execute.
    /// 3. Frontend — fetch1 / fetch2 / decode / rename.
    pub fn tick(&mut self, state: &mut CoreCtx<'_>) {
        drain(self, &mut state.stage());

        let frontend_empty = self.frontend.is_empty()
            && self.rename_output.is_empty()
            && !self.engine.common().fetch.is_waiting();
        self.engine.common_mut().frontend_empty = frontend_empty;
        // The engine dispatches the bundle, or leaves it when a trap or squash
        // makes it wrong-path; the redirect below discards the frontend then.
        let mut dispatch = self.rename_output.take(state.cycle);
        self.engine.tick(state, &mut dispatch, &mut self.redirect);

        let fetch_squashing = match self.redirect.take() {
            Some(pc) => {
                self.discard_frontend_speculation(state);
                self.frontend.fetch_pc = pc;
                self.recovering_from_redirect = true;
                self.engine.fetch_squashes_for_a_cycle()
            }
            None => false,
        };

        if !fetch_squashing && state.peek_exit().is_none() && !state.hart.wfi_waiting {
            self.frontend.tick(&mut state.stage(), &mut self.engine, &mut self.rename_output);
        }
        self.count_control_stall(state);
    }

    /// Counts a cycle in which rename has yet to hand on anything from the
    /// path a backend redirect started.
    fn count_control_stall(&mut self, state: &mut CoreCtx<'_>) {
        if !self.recovering_from_redirect || state.hart.wfi_waiting {
            return;
        }
        if self.rename_output.is_empty() {
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_control).inc();
        } else {
            self.recovering_from_redirect = false;
        }
    }

    /// Drops wrong-path frontend speculation: the fetch latches, in-flight
    /// fetches and fetch walks, and the fetch reorder buffer. Backend
    /// resources (outstanding loads, stores and their walks) belong to ROB
    /// entries a redirect leaves intact, so they survive.
    fn discard_frontend_speculation(&mut self, state: &mut CoreCtx<'_>) {
        let squashed = self
            .frontend
            .seqs()
            .chain(self.rename_output.entries().map(|entry| entry.seq))
            .chain(self.engine.common().in_flight_fetch_seqs())
            .collect::<Vec<_>>();
        for seq in squashed {
            state.trace_left(seq, Departure::Squashed);
        }
        self.frontend.flush();
        self.rename_output.clear();
        let common = self.engine.common_mut();
        common.drop_fetches();
        common.vector_config_unresolved = false;
    }

    /// One cycle of draining a flushed pipeline: takes the memory system's
    /// acknowledgements and sends the next committed store's write. Returns
    /// whether any committed store has yet to finish writing.
    pub fn drain_writes(&mut self, state: &mut CoreCtx<'_>) -> bool {
        drain(self, &mut state.stage());
        self.engine.send_committed_write(state)
    }

    /// Flush the entire pipeline; fetch restarts at the hart's
    /// architectural PC.
    pub fn flush(&mut self, state: &mut CoreCtx<'_>) {
        self.frontend.fetch_pc = state.hart.pc;
        self.redirect = None;
        self.recovering_from_redirect = false;
        self.discard_frontend_speculation(state);
        let common = self.engine.common_mut();
        common.mailbox.clear();
        // Fetch walks too, which the engine's flush leaves to the frontend.
        // Committed stores' writes are still in the memory system; their
        // acknowledgements free store-buffer slots.
        common.memory.outstanding_walks.clear();
        common.commit.trap = TrapProgress::None;
        self.engine.flush(state);
    }
}

/// Type-erased pipeline stored per core on the simulator.
#[derive(Debug)]
pub enum PipelineDispatch {
    /// In-order pipeline.
    InOrder(Box<Pipeline<InOrderEngine>>),
    /// Out-of-order pipeline.
    OutOfOrder(Box<Pipeline<O3Engine>>),
}

/// Runs `$body` with `$p` bound to whichever pipeline `$dispatch` holds.
macro_rules! on_pipeline {
    ($dispatch:expr, $p:ident => $body:expr) => {
        match $dispatch {
            PipelineDispatch::InOrder($p) => $body,
            PipelineDispatch::OutOfOrder($p) => $body,
        }
    };
}

impl PipelineDispatch {
    /// Builds the configured backend for `core`, fetching from `pc`.
    pub fn new(config: &Config, core: &CoreTopology, pc: VirtAddr) -> Self {
        let l1i = core.cache(PrivateCache::L1I);
        let l1d = core.cache(PrivateCache::L1D);
        match config.pipeline.backend {
            BackendKind::InOrder => Self::InOrder(Box::new(Pipeline::new(
                InOrderEngine::new(config, core.pipeline_id, l1i, l1d),
                config,
                pc,
            ))),
            BackendKind::OutOfOrder => Self::OutOfOrder(Box::new(Pipeline::new(
                O3Engine::new(config, core.pipeline_id, l1i, l1d),
                config,
                pc,
            ))),
        }
    }

    /// Whether a committed store has yet to finish writing: one in the
    /// store buffer, the vector store buffer or the write-combining buffer.
    pub fn has_committed_writes(&self, units: &CoreUnits) -> bool {
        let (store_buffer, vec_store_buffer) =
            on_pipeline!(self, p => (p.engine.store_buffer(), p.engine.vec_store_buffer()));
        store_buffer.has_committed_stores()
            || vec_store_buffer.has_committed_stores()
            || units.wcb.has_pending()
    }

    /// See [`Pipeline::is_idle`].
    pub fn is_idle(&self, hart: &Hart, units: &CoreUnits) -> bool {
        on_pipeline!(self, p => p.is_idle(hart, units))
    }

    /// Run one cycle.
    pub fn tick(&mut self, state: &mut CoreCtx<'_>) {
        on_pipeline!(self, p => p.tick(state));
    }

    /// Places an inbound packet into the pipeline's mailbox.
    pub fn deliver(&mut self, source: ComponentId, packet: Packet) {
        on_pipeline!(self, p => p.deliver(source, packet));
    }

    #[cfg(test)]
    /// The PC fetch continues from.
    #[must_use]
    pub const fn fetch_pc(&self) -> VirtAddr {
        on_pipeline!(self, p => p.frontend.fetch_pc)
    }

    #[cfg(test)]
    /// Whether a device read has been sent and not yet answered.
    pub fn device_read_in_flight(&self) -> bool {
        let common = on_pipeline!(self, p => p.engine.common());
        common.memory.outstanding_loads.values().any(|load| load.side_effecting)
    }

    #[cfg(test)]
    /// Makes the empty ROB allocate tags from `next` on.
    pub fn start_rob_tags_at(&mut self, next: RobTag) {
        on_pipeline!(self, p => p.engine.rob.start_tags_at(next));
    }

    /// Points fetch at `pc` without disturbing anything in flight; for
    /// initialisation, after the hart's architectural PC has been set.
    pub const fn restart_fetch_at(&mut self, pc: VirtAddr) {
        on_pipeline!(self, p => p.frontend.fetch_pc = pc);
    }

    /// Flush.
    pub fn flush(&mut self, state: &mut CoreCtx<'_>) {
        on_pipeline!(self, p => p.flush(state));
    }

    /// See [`Pipeline::drain_writes`].
    pub fn drain_writes(&mut self, state: &mut CoreCtx<'_>) -> bool {
        on_pipeline!(self, p => p.drain_writes(state))
    }

    /// Capture a point-in-time snapshot of all inter-stage latch contents.
    pub fn snapshot(&self, width: usize) -> LatchSnapshot {
        on_pipeline!(self, p => p.snapshot(width))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uarch::pipeline::backend::inorder::InOrderEngine;
    use crate::uarch::pipeline::frontend::Frontend;

    fn squash(
        keep: u32,
        cause: crate::uarch::pipeline::squash::SquashCause,
        target: u64,
    ) -> PendingSquash {
        PendingSquash {
            keep_tag: Some(RobTag::new(keep)),
            redirect: crate::uarch::pipeline::squash::Redirect::to(VirtAddr::new(target), cause),
            apply_at: 10,
        }
    }

    #[test]
    fn a_commit_reaches_the_predictor_a_cycle_later() {
        let mut predictor =
            crate::uarch::bpred::BranchPredictor::new(&crate::config::Config::minimal());
        let mut common = BackendCommon::default();
        common.note_committed(InstSeq::new(4), 10);

        common.deliver_commit_notices(&mut predictor, 10);
        let pending_at_commit = common.commit.commit_notices.len();
        common.deliver_commit_notices(&mut predictor, 11);

        assert_eq!((pending_at_commit, common.commit.commit_notices.len()), (1, 0));
    }

    #[test]
    fn a_waiting_squash_holds_commit_notices_back() {
        use crate::uarch::pipeline::squash::SquashCause;
        let mut predictor =
            crate::uarch::bpred::BranchPredictor::new(&crate::config::Config::minimal());
        let mut common = BackendCommon::default();
        common.note_committed(InstSeq::new(4), 10);
        common.request_squash(squash(5, SquashCause::Branch, 0x2000));

        common.deliver_commit_notices(&mut predictor, 20);

        assert_eq!(common.commit.commit_notices.len(), 1);
    }

    #[test]
    fn a_mispredict_filed_after_a_violation_behind_it_still_redirects() {
        use crate::uarch::pipeline::squash::SquashCause;
        let mut common = BackendCommon::default();

        common.request_squash(squash(5, SquashCause::MemoryOrder, 0x1000));
        common.request_squash(squash(5, SquashCause::Branch, 0x2000));

        let taken = common.take_due_squash(10).map(|s| s.redirect.target);
        assert_eq!(taken, Some(VirtAddr::new(0x2000)));
    }

    #[test]
    fn a_violation_filed_after_a_mispredict_does_not_replace_it() {
        use crate::uarch::pipeline::squash::SquashCause;
        let mut common = BackendCommon::default();

        common.request_squash(squash(5, SquashCause::Branch, 0x2000));
        common.request_squash(squash(5, SquashCause::MemoryOrder, 0x1000));

        let taken = common.take_due_squash(10).map(|s| s.redirect.target);
        assert_eq!(taken, Some(VirtAddr::new(0x2000)));
    }

    #[test]
    fn test_backend_type_default() {
        assert_eq!(BackendKind::default(), BackendKind::InOrder);
    }

    #[test]
    fn test_pipeline_dispatch_inorder_tick_flush_snapshot() {
        let config = crate::config::Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        let frontend = Frontend::new(state.hart.pc, &config.pipeline);
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let pipeline = Pipeline {
            frontend,
            engine,
            rename_output: Latch::new(config.pipeline.rename_issue_latency - 1),
            redirect: None,
            recovering_from_redirect: false,
        };
        let mut dispatch = PipelineDispatch::InOrder(Box::new(pipeline));

        dispatch.tick(&mut state);
        dispatch.flush(&mut state);
        let snapshot = dispatch.snapshot(1);
        assert_eq!(snapshot.width, 1);
    }
}
