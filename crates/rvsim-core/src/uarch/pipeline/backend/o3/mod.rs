//! Out-of-Order (O3) backend: issue queue with wakeup/select, out-of-order execution.
//!
//! The O3 backend reuses shared pipeline stages (Memory1, Memory2, Writeback,
//! Commit) and shared hardware units (ALU, FPU, BRU), but has its own:
//! - **`IssueQueue`**: CAM-style with wakeup/select (vs FIFO for in-order)
//! - **`execute_one()`**: single-instruction execute (vs batch for in-order)

mod complete;
pub mod execute;
pub mod fu_pool;
mod issue;
pub mod issue_queue;
mod memory;
mod rename;

pub use self::rename::{O3Entry, RenamedRegisters};
mod serialize;
mod squash;

use crate::common::VirtAddr;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VRegIdx;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::mdp::MemDepUnit;
use crate::uarch::pipeline::backend::shared::commit;
use crate::uarch::pipeline::backend::shared::vec_mem::{VecMemInflight, VecMemMicroOp};
use crate::uarch::pipeline::engine::{BundleHazard, DataCacheFlush, ExecutionEngine};
use crate::uarch::pipeline::latches::{
    ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry, SourceReadiness,
};
use crate::uarch::pipeline::lsq::load_queue::{LoadQueue, OrderViolation};
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::checkpoint::CheckpointTable;
use crate::uarch::pipeline::rename::free_list::{FreeList, PhysRegister};
use crate::uarch::pipeline::rename::map::RenameMap;
use crate::uarch::pipeline::rename::prf::{PhysReg, PhysRegFile};
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysRegFile;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, Rob};
use crate::uarch::pipeline::squash::SquashScope;
use crate::uarch::vector::chaining::VecPendingResult;
use crate::uarch::vector::lane_model::NumLanes;

use self::fu_pool::FuPool;
use self::issue_queue::IssueQueue;

/// Physical registers the integer and floating-point architectural state
/// holds: the GPRs first, then the FPRs.
const ARCH_SCALAR_REGS: usize = 2 * RegIdx::COUNT;
/// Physical vector registers the architectural v0..v31 hold.
const ARCH_VECTOR_REGS: usize = VRegIdx::COUNT;

/// A result a functional unit is still producing. Its dependents wake, and
/// its ROB entry completes, when `complete_cycle` arrives.
#[derive(Debug)]
pub struct PendingResult {
    /// The execute-stage result entry.
    pub entry: ExMem1Entry,
    /// Cycle at which the result is ready (wakeup fires at this cycle).
    pub complete_cycle: u64,
}

/// The free slots rename sees this cycle: the counts as they stood at the
/// end of the previous cycle, less what rename has taken since. Rename and
/// commit work in parallel, each from the state the other left a cycle
/// ago, as pipelined allocation bookkeeping does.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenameView {
    /// ROB entries rename may take this cycle.
    pub rob: usize,
    /// Issue-queue entries rename may take this cycle.
    pub iq: usize,
    /// Load-queue entries rename may take this cycle.
    pub lq: usize,
    /// Store-buffer entries rename may take this cycle.
    pub sq: usize,
}

/// Out-of-order execution engine.
#[derive(Debug)]
pub struct O3Engine {
    /// Reorder buffer.
    pub(crate) rob: Rob,
    /// Store buffer.
    store_buffer: StoreBuffer,
    /// Load queue for memory ordering violation detection.
    load_queue: LoadQueue,
    /// Physical register file (64-bit values + ready bits).
    prf: PhysRegFile,
    /// Free list of available physical register indices.
    free_list: FreeList<PhysReg>,
    /// Speculative rename map: arch reg → physical reg.
    rename_map: RenameMap,
    /// Committed rename map — restored on full trap flush.
    committed_rename_map: RenameMap,
    /// CAM-style issue queue with wakeup/select.
    issue_queue: IssueQueue,
    /// Functional unit pool for structural hazard modeling.
    fu_pool: FuPool,
    /// Non-memory results their unit is still producing.
    pending_results: Vec<PendingResult>,
    /// Memory ops whose address unit is still generating their address.
    pending_addresses: Vec<PendingResult>,
    /// Instructions renamed and dispatched per cycle.
    rename_width: usize,
    /// Instructions issued per cycle.
    issue_width: usize,
    /// Instructions retired per cycle.
    commit_width: usize,
    /// Results written back per cycle (gem5's `wbWidth`); the rest wait
    /// for a later cycle's slots.
    writeback_width: usize,
    /// Maximum loads issued per cycle.
    load_ports: usize,
    /// Maximum stores issued per cycle.
    store_ports: usize,
    /// Execute -> Memory1 latch.
    execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 -> Memory2 latch.
    mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 -> Writeback latch.
    mem2_wb: Vec<Mem2WbEntry>,
    /// Memory dependence unit for load-store ordering speculation.
    mdp: MemDepUnit,
    /// Checkpoint table for O(1) branch misprediction recovery.
    checkpoints: CheckpointTable,
    /// ROB entries commit squashes per cycle (gem5's `squashWidth`).
    squash_width: usize,
    /// The free slots rename works from this cycle.
    rename_view: RenameView,
    /// Store-buffer slots free when last cycle's engine tick began, before
    /// this cycle's write acknowledgements freed more.
    sq_free_last_cycle: usize,
    /// Stores renamed last cycle, after that count was taken.
    stores_renamed_last_cycle: usize,
    /// Stores renamed so far this cycle.
    stores_renamed_this_cycle: usize,
    /// Cycles commit still spends squashing the ROB; rename is blocked
    /// while it does.
    squash_stall_remaining: u64,
    /// Whether rename holds the instruction after a serializing one.
    serialization: serialize::Serialization,
    /// Cycles from a result that redirects to the squash being taken.
    redirect_latency: u64,
    /// Vector physical register file (VLEN-bit storage per register + ready bits).
    vec_prf: VecPhysRegFile,
    /// Vector physical register free list.
    vec_free_list: FreeList<VecPhysReg>,
    /// Pending vector results tracking chaining wakeup and completion.
    vec_pending: Vec<VecPendingResult>,
    /// Number of vector execution lanes (derived from VLEN / 64, min 1).
    num_vec_lanes: NumLanes,
    /// Pending vector memory micro-ops waiting to enter the memory pipeline.
    vec_mem_pending: std::collections::VecDeque<VecMemMicroOp>,
    /// Tracks in-flight vector memory instructions and their remaining element count.
    vec_mem_inflight: Vec<VecMemInflight>,
    /// Dedicated store buffer for in-flight vector stores. See
    /// `core::pipeline::vec_store_buffer` for forwarding/drain semantics.
    vec_store_buffer: VecStoreBuffer,
    /// In-flight memory bookkeeping: mailbox + outstanding tables + routing IDs.
    common: crate::uarch::pipeline::engine::BackendCommon,
}

impl O3Engine {
    /// Takes the free-slot view rename works from this cycle: ROB, issue-
    /// and load-queue slots as they stand before this cycle's commit and
    /// issue free more, less the `pending_dispatch` instructions renamed
    /// last cycle that have yet to enter the issue queue; store-buffer
    /// slots as they stood a cycle ago, since this cycle's write
    /// acknowledgements have already landed, less the stores renamed since.
    fn take_rename_view(&mut self, pending_dispatch: usize) {
        self.rename_view = RenameView {
            rob: self.rob.free_slots(),
            iq: self.issue_queue.available_slots().saturating_sub(pending_dispatch),
            lq: self.load_queue.free_slots(),
            sq: self.sq_free_last_cycle.saturating_sub(self.stores_renamed_last_cycle),
        };
        self.sq_free_last_cycle = self.store_buffer.free_slots();
        self.stores_renamed_last_cycle = self.stores_renamed_this_cycle;
        self.stores_renamed_this_cycle = 0;
    }

    /// Creates a new O3 engine from config and routing IDs.
    pub fn new(
        config: &Config,
        pipeline_id: crate::sim::components::PipelineId,
        l1_i_id: crate::sim::components::CacheId,
        l1_d_id: crate::sim::components::CacheId,
    ) -> Self {
        let rob_size = config.pipeline.rob_size;
        let prf_gpr_size = config.pipeline.prf_gpr_size;
        let prf_fpr_size = config.pipeline.prf_fpr_size;
        let prf_total = prf_gpr_size + prf_fpr_size;
        let mut prf = PhysRegFile::new(prf_total);
        prf.mark_arch_ready(ARCH_SCALAR_REGS);

        let fu_pool = FuPool::new(&config.pipeline.fu_config);

        Self {
            rob: Rob::new(rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            load_queue: LoadQueue::new(config.pipeline.load_queue_size),
            prf,
            free_list: FreeList::new(prf_total, ARCH_SCALAR_REGS),
            rename_map: RenameMap::new(),
            committed_rename_map: RenameMap::new(),
            issue_queue: IssueQueue::new(config.pipeline.issue_queue_size),
            fu_pool,
            pending_results: Vec::new(),
            pending_addresses: Vec::new(),
            rename_width: config.pipeline.rename_width(),
            issue_width: config.pipeline.issue_width(),
            commit_width: config.pipeline.commit_width(),
            writeback_width: config.pipeline.writeback_width(),
            load_ports: config.pipeline.load_ports,
            store_ports: config.pipeline.store_ports,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            mdp: MemDepUnit::new(config),
            checkpoints: CheckpointTable::new(config.pipeline.checkpoint_count),
            squash_width: config.pipeline.squash_width,
            rename_view: RenameView::default(),
            sq_free_last_cycle: config.pipeline.store_buffer_size,
            stores_renamed_last_cycle: 0,
            stores_renamed_this_cycle: 0,
            squash_stall_remaining: 0,
            serialization: serialize::Serialization::Off,
            redirect_latency: config.pipeline.redirect_latency(),
            vec_prf: {
                let prf_vpr_size = config.pipeline.prf_vpr_size;
                let mut vprf = VecPhysRegFile::new(prf_vpr_size, config.pipeline.vlen);
                vprf.mark_arch_ready(ARCH_VECTOR_REGS);
                vprf
            },
            vec_free_list: FreeList::new(config.pipeline.prf_vpr_size, ARCH_VECTOR_REGS),
            vec_pending: Vec::new(),
            num_vec_lanes: NumLanes::new(config.pipeline.vector_lanes()),
            vec_mem_pending: std::collections::VecDeque::new(),
            vec_mem_inflight: Vec::new(),
            vec_store_buffer: VecStoreBuffer::new(
                config.pipeline.vec_store_buffer_size,
                config.pipeline.vec_store_forwarding,
            ),
            common: crate::uarch::pipeline::engine::BackendCommon {
                pipeline_id,
                l1_i_id,
                l1_d_id,
                ..crate::uarch::pipeline::engine::BackendCommon::default()
            },
        }
    }

    /// Copies the hart's architectural registers into the physical
    /// registers its committed rename map names: at reset, and whenever a
    /// debugger changes a register while nothing is in flight.
    pub fn sync_arch_regs(&mut self, state: &crate::uarch::ctx::CoreCtx<'_>) {
        use crate::isa::reg::ArchReg;
        for reg in RegIdx::all().skip(1) {
            let phys = self.committed_rename_map.get(ArchReg::Int(reg));
            self.prf.write(phys, state.hart.regs.read(reg));
        }
        for reg in RegIdx::all() {
            let phys = self.committed_rename_map.get(ArchReg::Fp(reg));
            self.prf.write(phys, state.hart.regs.read_f(reg));
        }
        for vreg in VRegIdx::all() {
            let bytes = state.hart.regs.vpr().read_bytes(vreg);
            self.vec_prf.write_bytes(self.committed_rename_map.get_vec(vreg), bytes);
        }
    }

    /// Checks that every physical register is free, holds architectural
    /// state, or is the destination of an instruction still in the ROB.
    fn debug_assert_registers_conserved(&self) {
        if !cfg!(debug_assertions) {
            return;
        }
        let in_flight = self.rob.iter().filter(|entry| !entry.dst.phys.is_hardwired()).count();
        debug_assert_eq!(
            self.free_list.available() + ARCH_SCALAR_REGS + in_flight,
            self.prf.capacity(),
            "PRF register leak: {} free + {ARCH_SCALAR_REGS} architectural + {in_flight} in flight",
            self.free_list.available(),
        );
        let vec_in_flight: usize =
            self.rob.iter().map(|entry| usize::from(entry.vec_dst_count)).sum();
        debug_assert_eq!(
            self.vec_free_list.available() + ARCH_VECTOR_REGS + vec_in_flight,
            self.vec_prf.capacity(),
            "vector PRF register leak: {} free + {ARCH_VECTOR_REGS} architectural + {vec_in_flight} in flight",
            self.vec_free_list.available(),
        );
    }
}

impl ExecutionEngine for O3Engine {
    type Regs = RenamedRegisters;

    fn tick(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<O3Entry>,
        redirect: &mut Option<VirtAddr>,
    ) {
        let now = state.cycle;

        self.take_rename_view(rename_output.len());
        self.count_squash_stall(state);
        if let Some(squash) = self.common.take_due_squash(now) {
            self.apply_squash(state, squash, redirect);
            rename_output.clear();
        }
        self.common.commit.head_at_cycle_start = HeadAtCycleStart::latch(&self.rob);
        if self.retire(state, redirect) {
            return;
        }
        self.serialization.observe(self.rob.is_empty(), now);

        let memory2_violation = self.memory2(state);
        self.retire_vector_memory_results();
        let mut writeback_slots = self.writeback_memory_results(state);
        let memory1_violation = self.memory1(state, now);
        self.squash_on_violation(
            state,
            now,
            OrderViolation::older(memory2_violation, memory1_violation),
        );

        let memory_blocked = self.note_memory_backpressure(state);
        writeback_slots -= self.writeback_forwarded_loads(state, writeback_slots);
        writeback_slots -= self.writeback_finished_results(state, now, writeback_slots);
        self.issue_vec_mem_waves();
        self.send_vector_memory_micro_ops();
        self.complete_vector_results(now, writeback_slots);

        self.issue(state, now, memory_blocked);
        self.dispatch(state, rename_output);
        self.publish_mdp_stats(state);
    }

    fn can_accept(&self) -> usize {
        // Rename waits while commit squashes the ROB.
        if self.squash_stall_remaining > 0 {
            return 0;
        }
        // Slots only some instructions need are checked per instruction.
        self.rename_view.rob.min(self.rename_view.iq).min(self.rename_width)
    }

    fn flush(&mut self, state: &mut CoreCtx<'_>) {
        self.squash_in_flight(state, SquashScope::All);
        self.serialization = serialize::Serialization::Off;
        self.common.commit.data_cache_flush = DataCacheFlush::Idle;
        self.rename_map = self.committed_rename_map.clone();
        self.checkpoints.flush_all();
        // Caller sets squash_stall_remaining after this; not cleared here.
        self.common.flush_predictions(&mut state.core.branch_predictor);
        self.debug_assert_registers_conserved();
    }

    fn rob(&self) -> &Rob {
        &self.rob
    }

    fn store_buffer(&self) -> &StoreBuffer {
        &self.store_buffer
    }

    fn store_buffer_mut(&mut self) -> &mut StoreBuffer {
        &mut self.store_buffer
    }

    fn vec_store_buffer(&self) -> &VecStoreBuffer {
        &self.vec_store_buffer
    }

    fn vec_store_buffer_mut(&mut self) -> &mut VecStoreBuffer {
        &mut self.vec_store_buffer
    }

    fn rename(
        &mut self,
        state: &mut crate::uarch::ctx::StageCtx<'_>,
        decoded: crate::uarch::pipeline::latches::IdExEntry,
    ) -> crate::uarch::pipeline::engine::Renamed<RenamedRegisters> {
        self.rename_one(state, decoded)
    }

    fn send_committed_write(&mut self, state: &mut CoreCtx<'_>) -> bool {
        commit::send_one_write(
            state,
            &mut self.common,
            &mut self.store_buffer,
            &mut self.vec_store_buffer,
        );
        commit::committed_writes_pending(state, &self.store_buffer, &self.vec_store_buffer)
    }

    fn execute_mem1_mut(&mut self) -> &mut Vec<crate::uarch::pipeline::latches::ExMem1Entry> {
        &mut self.execute_mem1
    }

    fn mem1_mem2_mut(&mut self) -> &mut Vec<crate::uarch::pipeline::latches::Mem1Mem2Entry> {
        &mut self.mem1_mem2
    }

    fn issue_snapshot(&self) -> Vec<RenameIssueEntry<SourceReadiness>> {
        self.issue_queue.queue_snapshot()
    }

    fn execute_mem1(&self) -> &[crate::uarch::pipeline::latches::ExMem1Entry] {
        &self.execute_mem1
    }

    fn mem1_mem2(&self) -> &[crate::uarch::pipeline::latches::Mem1Mem2Entry] {
        &self.mem1_mem2
    }

    fn mem2_wb(&self) -> &[Mem2WbEntry] {
        &self.mem2_wb
    }

    fn common(&self) -> &crate::uarch::pipeline::engine::BackendCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut crate::uarch::pipeline::engine::BackendCommon {
        &mut self.common
    }

    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        Some(&mut self.load_queue)
    }

    fn bundle_hazard(&self) -> BundleHazard {
        BundleHazard::Renamed
    }

    fn fetch_squashes_for_a_cycle(&self) -> bool {
        true
    }

    fn is_recovering_from_squash(&self) -> bool {
        self.squash_stall_remaining > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::isa::reg::RegIdx;

    #[test]
    fn test_o3_engine_new_and_flush() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        let mut engine = O3Engine::new(
            &config,
            crate::sim::components::PipelineId::new(0),
            crate::sim::components::CacheId::new(0),
            crate::sim::components::CacheId::new(1),
        );
        assert_eq!(engine.rename_width, config.pipeline.rename_width());

        engine.flush(&mut state);
        assert_eq!(engine.execute_mem1.len(), 0);
    }

    #[test]
    fn test_o3_engine_sync_arch_regs() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let state = sys.core_ctx(0);
        let mut engine = O3Engine::new(
            &config,
            crate::sim::components::PipelineId::new(0),
            crate::sim::components::CacheId::new(0),
            crate::sim::components::CacheId::new(1),
        );

        state.hart.regs.write(RegIdx::new(1), 42);
        engine.sync_arch_regs(&state);

        assert_eq!(engine.prf.read(crate::uarch::pipeline::rename::prf::PhysReg::new(1)), 42);
    }
}
