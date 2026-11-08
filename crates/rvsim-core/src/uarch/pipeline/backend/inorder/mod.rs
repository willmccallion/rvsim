//! In-order backend: FIFO issue, single execution path.
//!
//! Implements the simple in-order pipeline with:
//! - [`InOrderIssueUnit`] — FIFO pass-through (no reordering).
//! - `InOrderExecuteUnit` — single ALU/FPU/BRU execution.
//!
//! The engine owns its in-flight memory bookkeeping
//! (`BackendCommon` = mailbox + `outstanding_loads` / stores / walks /
//! fetches + `next_req_id` + routing IDs). Memory1 runs inside `tick`,
//! reaching the outstanding tables and the event queue directly.

pub mod execute;
pub mod issue;
mod rename;

use crate::common::VirtAddr;
use crate::config::Config;
use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::exec::compute::vector::shadow::ElementWrite;
use crate::exec::execute::illegal_in_current_state;
use crate::isa::privileged::Trap;
use crate::isa::rvv::{ElemIdx, VRegIdx, parse_vtype};
use crate::sim::components::{CacheId, PipelineId};
use crate::uarch::ctx::CoreCtx;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::commit::{
    CommitEvent, CommitRegisters, CommitResources,
};
use crate::uarch::pipeline::backend::shared::flush_stats::count_flush;
use crate::uarch::pipeline::backend::shared::issue_stats::{IssueHold, count_issue_stalls};
use crate::uarch::pipeline::backend::shared::vec_mem::{
    VecMemInflight, expand_span, retire_access,
};
use crate::uarch::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::uarch::pipeline::engine::Renamed;
use crate::uarch::pipeline::engine::{
    BackendCommon, BundleHazard, DataCacheFlush, ExecutionEngine,
};
use crate::uarch::pipeline::latches::IdExEntry;
use crate::uarch::pipeline::latches::{
    ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry, SourceReadiness,
};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::scoreboard::Scoreboard;
use crate::uarch::pipeline::rename::vec_prf::VecRegGroup;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, Rob};
use crate::uarch::pipeline::squash::PendingSquash;
use crate::uarch::pipeline::squash::SquashScope;
use crate::uarch::pipeline::trace::Departure;

use self::issue::{
    InOrderEntry, InOrderIssueUnit, IssueContext, IssuedUnit, ProducerTags, Selection,
};
use crate::uarch::pipeline::backend::o3::fu_pool::FuPool;

/// A computed result waiting for its unit's latency to elapse.
#[derive(Debug)]
struct PendingResult {
    complete_cycle: u64,
    entry: ExMem1Entry,
}

/// Forwards each scalar load's value the cycle it is known, at memory2 or
/// when a store forwards it, so a dependent issues then rather than after
/// writeback, as Rocket bypasses its D-cache response into the next
/// instruction's execute.
fn forward_loads(rob: &mut Rob, delivered: &[Mem2WbEntry]) {
    for wb in delivered {
        let scalar_load = wb.op.ctrl.mem_read || wb.op.ctrl.atomic_op.is_some();
        if scalar_load && wb.op.trap.is_none() && wb.op.vec_mem.is_none() {
            rob.forward(wb.op.rob_tag, wb.load_data);
        }
    }
}

/// In-order execution engine.
#[derive(Debug)]
pub struct InOrderEngine {
    /// Reorder buffer.
    pub(crate) rob: Rob,
    /// Store buffer.
    store_buffer: StoreBuffer,
    /// Tag-based register scoreboard.
    scoreboard: Scoreboard,
    /// FIFO issue unit.
    issuer: InOrderIssueUnit,
    /// Functional units, taken at issue.
    fu_pool: FuPool,
    /// Results a unit has yet to deliver: forwarded to dependents and sent
    /// down the memory stages at `complete_cycle`.
    pending: Vec<PendingResult>,
    /// Instructions renamed and dispatched per cycle.
    rename_width: usize,
    /// Instructions issued per cycle.
    issue_width: usize,
    /// Instructions retired per cycle.
    commit_width: usize,
    /// Vector memory instructions in flight, with the element micro-ops
    /// not yet in the memory pipeline.
    vec_mem_inflight: Vec<VecMemInflight>,
    /// Buffered element data of vector stores, written to memory after
    /// commit.
    vec_store_buffer: VecStoreBuffer,
    /// Execute → Memory1 latch.
    execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 → Memory2 latch.
    mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 → Writeback latch.
    mem2_wb: Vec<Mem2WbEntry>,
    /// In-flight memory bookkeeping: mailbox + outstanding tables + routing IDs.
    common: BackendCommon,
    /// Cycles from a result that redirects to the squash being taken.
    redirect_latency: u64,
    /// Whether a renamed bundle issues the cycle it reaches the backend
    /// (`rename_issue_latency` of 1): dispatch runs before issue.
    dispatch_before_issue: bool,
    /// This cycle's redirect came from commit, which fetch learns of the
    /// cycle after.
    redirected_at_commit: bool,
}

impl InOrderEngine {
    /// Sends every result whose unit has finished to the memory stages and
    /// forwards its value so a dependent can issue this cycle.
    fn deliver_ready_results(&mut self, now: u64) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].complete_cycle > now {
                i += 1;
                continue;
            }
            let done = self.pending.swap_remove(i);
            self.rob.forward(done.entry.op.rob_tag, done.entry.register_value());
            self.execute_mem1.push(done.entry);
        }
    }

    /// Removes from every structure what `scope` squashes. Everything in
    /// the issue queue is younger than any executed instruction, so it all
    /// goes.
    fn squash_in_flight(&mut self, state: &mut CoreCtx<'_>, scope: SquashScope) {
        for seq in self.rob.seqs_after(scope.keep_tag()) {
            state.trace_left(seq, Departure::Squashed);
        }
        self.issuer.flush();
        self.rob.squash(scope);
        self.store_buffer.squash(scope);
        self.vec_store_buffer.squash(scope);
        self.common.squash(scope);
        self.fu_pool.release_squashed(state.cycle, |tag| !scope.keeps(tag));
        self.vec_mem_inflight.retain(|m| scope.keeps(m.rob_tag));
        self.pending.retain(|p| scope.keeps(p.entry.op.rob_tag));
        self.execute_mem1.retain(|e| scope.keeps(e.op.rob_tag));
        self.mem1_mem2.retain(|e| scope.keeps(e.op.rob_tag));
        self.mem2_wb.retain(|e| scope.keeps(e.op.rob_tag));
        self.scoreboard.rebuild_from_rob(&self.rob);
    }

    /// Takes a squash: drops everything younger than its kept tag (or the
    /// whole window when that tag has already retired) and redirects fetch.
    fn apply_squash(
        &mut self,
        state: &mut CoreCtx<'_>,
        squash: PendingSquash,
        redirect: &mut Option<VirtAddr>,
    ) {
        let keep_tag = squash.keep_tag.filter(|tag| self.rob.find_entry(*tag).is_some());
        let scope = SquashScope::keeping(keep_tag);
        count_flush(state, squash.redirect.cause.into(), self.rob.iter_squashed(scope).count());
        let keep_seq = keep_tag.and_then(|tag| self.rob.find_entry(tag)).map(|entry| entry.seq);
        self.squash_in_flight(state, scope);

        *redirect = Some(squash.redirect.target);
        let now = state.cycle;
        self.common.squash_predictions(&mut state.core.branch_predictor, &squash, keep_seq, now);
    }

    /// Files each executed result: memory ops and `vsetvl` go straight to
    /// memory1 (a memory op's unit is the address generator, whose latency
    /// is modelled downstream); everything else, vector arithmetic included,
    /// waits for its unit's latency in `pending`.
    fn hold_results(&mut self, results: Vec<ExMem1Entry>, units: &[IssuedUnit], now: u64) {
        for entry in results {
            let is_mem = entry.op.ctrl.uses_memory_pipeline();
            let unit = units.iter().find(|u| u.tag == entry.op.rob_tag);
            match unit {
                Some(unit) if !is_mem && !entry.op.ctrl.is_vector_config() => {
                    self.pending.push(PendingResult {
                        complete_cycle: unit.complete_cycle.max(now + 1),
                        entry,
                    });
                }
                _ => self.execute_mem1.push(entry),
            }
        }
    }

    /// Starts a vector load or store: its element micro-ops are generated
    /// from the architectural registers, which are current because a
    /// vector instruction issues only from the ROB head.
    fn start_vec_mem_op(&mut self, state: &CoreCtx<'_>, entry: &InOrderEntry) {
        use crate::exec::compute::vector::mem::{check_vec_mem_emul, element_accesses};
        if illegal_in_current_state(state.hart, &entry.inst) {
            let trap = Trap::IllegalInstruction(entry.inst.bits);
            self.rob.fault(entry.rob_tag, trap);
            return;
        }
        let vec_op = entry.inst.ctrl.vec_op;
        let is_store = is_vec_store(vec_op);
        let vtype = parse_vtype(state.hart.csrs.vtype);
        if let Err(trap) = check_vec_mem_emul(entry.inst.bits, &entry.inst.ctrl, vtype) {
            self.rob.fault(entry.rob_tag, trap);
            return;
        }
        let addresses = element_accesses(
            state.hart.regs.vpr(),
            entry.inst.rv1,
            entry.inst.rv2 as i64,
            &entry.inst.ctrl,
            state.hart.csrs.vector_config(),
        );
        if is_store {
            self.vec_store_buffer.set_expected_elements(entry.rob_tag, addresses.len());
        }
        if addresses.is_empty() {
            self.rob.complete(entry.rob_tag, 0);
            return;
        }
        let parent = ExMem1Entry::from_issue(entry, entry.inst.rv1, entry.inst.rv2);
        let width = state.config.pipeline.vector_mem_width_bytes();
        // Element writes land in the architectural registers at commit, so
        // there is no physical destination group to wake.
        self.vec_mem_inflight.push(VecMemInflight::plan(
            &parent,
            addresses,
            width,
            VecRegGroup::EMPTY,
        ));
    }

    /// Moves vector memory micro-ops into the memory pipeline, as many per cycle
    /// as the load and store ports allow.
    fn issue_vec_mem_elements(&mut self, state: &CoreCtx<'_>) {
        let mut loads_left = state.config.pipeline.load_ports;
        let mut stores_left = state.config.pipeline.store_ports;
        for inflight in &mut self.vec_mem_inflight {
            while let Some(front) = inflight.pending_micro_ops.front() {
                let ports_left = if front.is_store { &mut stores_left } else { &mut loads_left };
                if *ports_left == 0 {
                    return;
                }
                *ports_left -= 1;
                let Some(mop) = inflight.pending_micro_ops.pop_front() else { break };
                self.execute_mem1.push(mop.entry);
            }
        }
    }

    /// Retires the vector memory micro-ops that reached writeback: a load's
    /// element data is filed on its ROB entry for commit to land, and the
    /// instruction completes with its last micro-op.
    fn retire_vec_mem_elements(&mut self, state: &CoreCtx<'_>) {
        let entries = std::mem::take(&mut self.mem2_wb);
        for wb in entries {
            let Some(access) = wb.op.vec_mem.as_ref() else {
                self.mem2_wb.push(wb);
                continue;
            };
            let retired = retire_access(&wb, access, &mut self.vec_mem_inflight, &mut self.rob);
            let vlen_bits = state.hart.regs.vpr().vlen().bits();
            for value in retired.writes {
                let elems_per_reg = (vlen_bits / (value.eew.bytes() * 8)).max(1);
                let write = ElementWrite {
                    reg: VRegIdx::new(wb.op.ctrl.vd.as_u8() + value.dest_slot as u8),
                    index: ElemIdx::new(value.elem_idx.as_usize() % elems_per_reg),
                    eew: value.eew,
                    data: value.value,
                };
                self.rob.push_vec_element_write(wb.op.rob_tag, write);
            }
            if retired.completed {
                self.vec_mem_inflight.retain(|m| m.rob_tag != wb.op.rob_tag);
            }
        }
    }

    /// Creates a new in-order engine from config and routing IDs.
    pub fn new(
        config: &Config,
        pipeline_id: PipelineId,
        l1_i_id: CacheId,
        l1_d_id: CacheId,
    ) -> Self {
        let common = BackendCommon { pipeline_id, l1_i_id, l1_d_id, ..BackendCommon::default() };
        Self {
            rob: Rob::new(config.pipeline.rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            scoreboard: Scoreboard::new(),
            issuer: InOrderIssueUnit::new(config.pipeline.rob_size),
            fu_pool: FuPool::new(&config.pipeline.fu_config),
            pending: Vec::new(),
            rename_width: config.pipeline.rename_width(),
            issue_width: config.pipeline.issue_width(),
            commit_width: config.pipeline.commit_width(),
            vec_mem_inflight: Vec::new(),
            vec_store_buffer: VecStoreBuffer::new(
                config.pipeline.vec_store_buffer_size,
                config.pipeline.vec_store_forwarding,
            ),
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            common,
            redirect_latency: config.pipeline.redirect_latency(),
            dispatch_before_issue: config.pipeline.rename_issue_latency == 1,
            redirected_at_commit: false,
        }
    }
}

impl InOrderEngine {
    /// Commits this cycle's instructions; true when a trap or a
    /// re-execution flushed the pipeline and redirected fetch.
    fn retire(&mut self, state: &mut CoreCtx<'_>, redirect: &mut Option<VirtAddr>) -> bool {
        let commit_event = commit::commit_stage(
            state,
            CommitResources {
                common: &mut self.common,
                rob: &mut self.rob,
                store_buffer: &mut self.store_buffer,
                vec_store_buffer: &mut self.vec_store_buffer,
                width: self.commit_width,
                registers: CommitRegisters::Scoreboard(&mut self.scoreboard),
            },
        );
        let Some(event) = commit_event else {
            return false;
        };
        count_flush(state, (&event).into(), self.rob.len());
        self.flush(state);
        let target = match event {
            CommitEvent::Trap(trap, pc) => {
                state.trap(&trap, pc);
                state.hart.pc
            }
            CommitEvent::ReExecute(pc, _) | CommitEvent::SquashAfter(pc) => {
                state.hart.pc = pc;
                pc
            }
        };
        *redirect = Some(target);
        self.redirected_at_commit = true;
        true
    }

    /// Runs memory2, then memory1, so an entry moves one stage a cycle.
    fn run_memory_stages(&mut self, state: &mut CoreCtx<'_>) {
        let _ = memory2::memory2_stage(
            &state.stage(),
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            memory2::MemoryQueues {
                store_buffer: &mut self.store_buffer,
                load_queue: None,
                vec_store_buffer: &mut self.vec_store_buffer,
            },
        );
        forward_loads(&mut self.rob, &self.mem2_wb);

        // Memory1 consumes execute_mem1, emits MemReq packets for misses,
        // and parks parked loads into self.common.outstanding_loads. SB
        // forwards and stores resolve straight into mem1_mem2.
        let mut input = std::mem::take(&mut self.execute_mem1);
        let resolved = memory1::memory1_stage(&mut state.stage(), self, &mut input);
        // Ops behind an unresolved translation walk go back; ops waiting on
        // a store-buffer drain live in `common.mem1_replay`.
        self.execute_mem1.extend(input);
        // A load forwarded at zero latency skips memory2 and writes back next cycle.
        let mut forwarded = std::mem::take(&mut self.common.memory.forwarded_results);
        forward_loads(&mut self.rob, &forwarded);
        self.mem2_wb.append(&mut forwarded);
        for span in resolved.expanded_spans {
            let _ = expand_span(&span, &mut self.vec_mem_inflight);
        }
    }

    /// Whether memory1 still holds entries, which issue must not overwrite;
    /// counted as a backpressure stall.
    fn note_memory_backpressure(&self, state: &mut CoreCtx<'_>) -> bool {
        let is_backpressured = !self.execute_mem1.is_empty();
        if is_backpressured {
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_backpressure).inc();
        }
        is_backpressured
    }

    /// Issues and executes this cycle's instructions, returning their
    /// results and the units they took. A branch that mispredicted asks
    /// for its squash once its unit finishes.
    fn issue(&mut self, state: &mut CoreCtx<'_>, now: u64) -> (Vec<ExMem1Entry>, Vec<IssuedUnit>) {
        let ctx = IssueContext {
            rob: &self.rob,
            head: self.common.commit.head_at_cycle_start,
            store_buffer: &self.store_buffer,
            vec_store_buffer: &self.vec_store_buffer,
            pending_squash: self.common.commit.pending_squash,
            now,
        };
        let Selection { issued, units, held } =
            self.issuer.select(self.issue_width, &ctx, &state.stage(), &mut self.fu_pool);
        count_issue_stalls(state, held == Some(IssueHold::Unit), !issued.is_empty(), held);
        for unit in &units {
            let stat = state.core.stat_paths.fu.all[unit.fu_type as usize];
            state.uncore.stats.counter(stat).add(unit.busy_cycles);
        }
        let (vec_mem, issued): (Vec<_>, Vec<_>) = issued.into_iter().partition(|entry| {
            is_vec_load(entry.inst.ctrl.vec_op) || is_vec_store(entry.inst.ctrl.vec_op)
        });
        for entry in &vec_mem {
            self.start_vec_mem_op(state, entry);
        }
        let executed = execute::execute_inorder(&mut state.stage(), &issued, &mut self.rob);
        for (tag, redirect) in executed.redirects {
            let complete_cycle =
                units.iter().find(|u| u.tag == tag).map_or(now + 1, |u| u.complete_cycle);
            self.common.request_squash(PendingSquash {
                keep_tag: Some(tag),
                redirect,
                apply_at: complete_cycle + self.redirect_latency,
            });
        }
        (executed.results, units)
    }
}

impl ExecutionEngine for InOrderEngine {
    type Regs = ProducerTags;

    fn tick(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<InOrderEntry>,
        redirect: &mut Option<VirtAddr>,
    ) {
        let now = state.cycle;
        self.redirected_at_commit = false;

        if let Some(squash) = self.common.take_due_squash(now) {
            self.apply_squash(state, squash, redirect);
            rename_output.clear();
        }
        self.common.commit.head_at_cycle_start = HeadAtCycleStart::latch(&self.rob);
        if self.retire(state, redirect) {
            return;
        }

        self.retire_vec_mem_elements(state);
        writeback::writeback_stage(&mut state.stage(), &mut self.mem2_wb, &mut self.rob);
        self.run_memory_stages(state);
        let is_backpressured = self.note_memory_backpressure(state);

        // A core whose rename hands straight to issue (Rocket's ID to EX)
        // dispatches before issue, so the bundle issues this cycle.
        if self.dispatch_before_issue {
            self.issuer.dispatch(std::mem::take(rename_output));
        }
        self.deliver_ready_results(now);
        // Issue would overwrite the entries memory1 still holds.
        let (results, units) =
            if is_backpressured { (Vec::new(), Vec::new()) } else { self.issue(state, now) };
        if results.iter().any(|r| r.op.ctrl.is_vector_config()) {
            self.common.vector_config_unresolved = false;
        }
        self.hold_results(results, &units, now);
        self.issue_vec_mem_elements(state);

        // Dispatch even during backpressure: skipping it lets rename_output outgrow issue capacity.
        self.issuer.dispatch(std::mem::take(rename_output));
    }

    fn can_accept(&self) -> usize {
        // A store's buffer slot is checked per instruction, at rename.
        let rob_free = self.rob.free_slots();
        let issue_free = self.issuer.available_slots();
        rob_free.min(issue_free).min(self.rename_width)
    }

    fn flush(&mut self, state: &mut CoreCtx<'_>) {
        self.squash_in_flight(state, SquashScope::All);
        self.common.commit.data_cache_flush = DataCacheFlush::Idle;
        self.common.flush_predictions(&mut state.core.branch_predictor);
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

    fn vec_store_buffer(&self) -> &VecStoreBuffer {
        &self.vec_store_buffer
    }

    fn vec_store_buffer_mut(&mut self) -> &mut VecStoreBuffer {
        &mut self.vec_store_buffer
    }

    fn bundle_hazard(&self) -> BundleHazard {
        BundleHazard::EndsBundle
    }

    /// Minor's fetch follows a branch the cycle it arrives.
    fn fetch_squashes_for_a_cycle(&self) -> bool {
        self.redirected_at_commit
    }

    fn is_recovering_from_squash(&self) -> bool {
        false
    }

    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        None
    }

    fn rename(&mut self, state: &mut StageCtx<'_>, decoded: IdExEntry) -> Renamed<ProducerTags> {
        self.rename_one(state, decoded)
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

    fn execute_mem1_mut(&mut self) -> &mut Vec<ExMem1Entry> {
        &mut self.execute_mem1
    }

    fn mem1_mem2_mut(&mut self) -> &mut Vec<Mem1Mem2Entry> {
        &mut self.mem1_mem2
    }

    fn issue_snapshot(&self) -> Vec<RenameIssueEntry<SourceReadiness>> {
        self.issuer.queue_snapshot()
    }

    fn execute_mem1(&self) -> &[ExMem1Entry] {
        &self.execute_mem1
    }

    fn mem1_mem2(&self) -> &[Mem1Mem2Entry] {
        &self.mem1_mem2
    }

    fn mem2_wb(&self) -> &[Mem2WbEntry] {
        &self.mem2_wb
    }

    fn common(&self) -> &BackendCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut BackendCommon {
        &mut self.common
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_inorder_engine_new() {
        let config = Config::minimal();
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        assert_eq!(engine.can_accept(), config.pipeline.width);
    }

    #[test]
    fn test_inorder_engine_flush() {
        let config = Config::minimal();
        let mut engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        engine.flush(&mut state);

        assert_eq!(engine.execute_mem1.len(), 0);
        assert_eq!(engine.mem1_mem2.len(), 0);
        assert_eq!(engine.mem2_wb.len(), 0);
    }
}
