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

use crate::config::Config;
use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::exec::compute::vector::shadow::ElementWrite;
use crate::exec::execute::unit_disabled;
use crate::isa::privileged::Trap;
use crate::isa::rvv::{ElemIdx, VRegIdx, parse_vtype};
use crate::sim::components::{CacheId, PipelineId};
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::backend::shared::commit::{
    CommitEvent, CommitRegisters, CommitResources,
};
use crate::uarch::pipeline::backend::shared::vec_mem::{
    VecMemInflight, expand_span, micro_ops_for, moves_in_spans, plan_accesses, retire_access,
};
use crate::uarch::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::uarch::pipeline::engine::{BackendCommon, ExecutionEngine};
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::scoreboard::Scoreboard;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rob::Rob;
use crate::uarch::pipeline::squash::{PendingSquash, SquashCause};

use self::issue::{InOrderIssueUnit, IssuedUnit};
use crate::exec::signals::ControlFlow;
use crate::uarch::pipeline::backend::o3::fu_pool::{FuPool, FuType};
use crate::uarch::pipeline::backend::shared::vec_mem::route_to_phys;

/// A computed result waiting for its unit's latency to elapse.
#[derive(Debug)]
struct PendingResult {
    complete_cycle: u64,
    fu_type: FuType,
    entry: ExMem1Entry,
}

/// The value a non-memory result forwards: the link address for a jump,
/// otherwise the ALU output.
const fn forwarded_value(entry: &ExMem1Entry) -> u64 {
    if matches!(entry.ctrl.control_flow, ControlFlow::Jump) {
        entry.pc.wrapping_add(entry.inst_size.as_u64())
    } else {
        entry.alu
    }
}

/// In-order execution engine.
#[derive(Debug)]
pub struct InOrderEngine {
    /// Reorder buffer.
    pub rob: Rob,
    /// Store buffer.
    pub store_buffer: StoreBuffer,
    /// Tag-based register scoreboard.
    pub scoreboard: Scoreboard,
    /// FIFO issue unit.
    pub issuer: InOrderIssueUnit,
    /// Functional units, taken at issue.
    pub fu_pool: FuPool,
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
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 → Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 → Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// In-flight memory bookkeeping: mailbox + outstanding tables + routing IDs.
    pub common: BackendCommon,
    /// Current cycle counter (used for debug stats; the simulator's cycle
    /// drives all timing).
    cycle: u64,
    /// Cycles from a result that redirects to the squash being taken.
    redirect_latency: u64,
}

impl InOrderEngine {
    /// Sends every result whose unit has finished to the memory stages and
    /// forwards its value so a dependent can issue this cycle.
    fn deliver_ready_results(&mut self, state: &mut CoreCtx<'_>, now: u64) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].complete_cycle > now {
                i += 1;
                continue;
            }
            let done = self.pending.swap_remove(i);
            state.uncore.stats.counter(state.core.stat_paths.fu.all[done.fu_type as usize]).inc();
            self.rob.forward(done.entry.rob_tag, forwarded_value(&done.entry));
            self.execute_mem1.push(done.entry);
        }
    }

    /// Takes a squash: drops everything younger than its kept tag (or the
    /// whole window when that tag has already retired) and redirects fetch.
    fn apply_squash(
        &mut self,
        state: &mut CoreCtx<'_>,
        squash: PendingSquash,
        redirect: &mut Option<u64>,
    ) {
        let paths = &state.core.stat_paths.pipeline;
        state.uncore.stats.counter(paths.flushes_total).inc();
        match squash.redirect.cause {
            SquashCause::Branch => state.uncore.stats.counter(paths.flushes_branch).inc(),
            SquashCause::System => state.uncore.stats.counter(paths.flushes_system).inc(),
            SquashCause::MemoryOrder | SquashCause::Coherence => {}
        }

        // Everything in the issue queue is younger than any executed instruction.
        self.issuer.flush();
        let keep_tag = squash.keep_tag.filter(|tag| self.rob.find_entry(*tag).is_some());
        let keep_seq = keep_tag.and_then(|tag| self.rob.find_entry(tag)).map(|entry| entry.seq);
        if let Some(keep_tag) = keep_tag {
            self.rob.flush_after(keep_tag);
            self.store_buffer.flush_after(keep_tag);
            self.vec_store_buffer.flush_after(keep_tag);
            self.common.squash_after(keep_tag);
        } else {
            self.rob.flush_all();
            self.store_buffer.flush_speculative();
            self.vec_store_buffer.flush_speculative();
            self.common.squash_all();
        }
        let survives = |tag: crate::uarch::pipeline::rob::RobTag| {
            keep_tag.is_some_and(|k| tag.is_older_or_eq(k))
        };
        self.vec_mem_inflight.retain(|m| survives(m.rob_tag));
        self.pending.retain(|p| survives(p.entry.rob_tag));
        self.execute_mem1.retain(|e| survives(e.rob_tag));
        self.mem1_mem2.retain(|e| survives(e.rob_tag));
        self.mem2_wb.retain(|e| survives(e.rob_tag));
        self.scoreboard.rebuild_from_rob(&self.rob);

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
            let is_mem = entry.ctrl.uses_memory_pipeline();
            let unit = units.iter().find(|u| u.tag == entry.rob_tag);
            match unit {
                Some(unit) if !is_mem && !entry.ctrl.vec_op.is_config() => {
                    self.pending.push(PendingResult {
                        complete_cycle: unit.complete_cycle.max(now + 1),
                        fu_type: unit.fu_type,
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
    fn start_vec_mem_op(&mut self, state: &CoreCtx<'_>, entry: &RenameIssueEntry) {
        use crate::exec::compute::vector::mem::{
            check_vec_mem_emul, element_accesses, vec_mem_dst_count,
        };
        if unit_disabled(state.hart, &entry.inst) {
            let trap = Trap::IllegalInstruction(entry.inst.bits);
            self.rob.fault(entry.rob_tag, trap, ExceptionStage::Execute);
            return;
        }
        let vec_op = entry.inst.ctrl.vec_op;
        let is_store = is_vec_store(vec_op);
        let vtype = parse_vtype(state.hart.csrs.vtype);
        if let Err(trap) = check_vec_mem_emul(entry.inst.bits, vec_op, &entry.inst.ctrl, vtype) {
            self.rob.fault(entry.rob_tag, trap, ExceptionStage::Execute);
            return;
        }
        let vd_count = if is_store {
            0
        } else {
            vec_mem_dst_count(
                vec_op,
                entry.inst.ctrl.vec_eew,
                vtype.vsew,
                vtype.vlmul,
                entry.inst.ctrl.vec_nf,
            )
        };
        let mut vd_regs = [VecPhysReg::ZERO; 8];
        for (offset, reg) in vd_regs.iter_mut().enumerate().take(vd_count as usize) {
            *reg = VecPhysReg::new(u16::from(entry.inst.ctrl.vd.as_u8()) + offset as u16);
        }
        let addresses = route_to_phys(
            element_accesses(
                state.hart.regs.vpr(),
                entry.inst.rv1,
                entry.inst.rv2 as i64,
                &entry.inst.ctrl,
                state.hart.csrs.vtype,
                state.hart.csrs.vl as usize,
                state.hart.csrs.vstart as usize,
                vec_op,
            ),
            &vd_regs,
            vd_count,
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
        let planned = plan_accesses(addresses, moves_in_spans(vec_op), width);
        let micro_ops = micro_ops_for(&parent, planned, is_store);
        self.vec_mem_inflight.push(VecMemInflight {
            rob_tag: entry.rob_tag,
            remaining: micro_ops.len(),
            vd_phys: vd_regs,
            vd_count,
            wakeup_fired: false,
            pending_micro_ops: micro_ops,
            trimmed_at: None,
            fault: None,
        });
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
            let Some(access) = wb.vec_mem.as_ref() else {
                self.mem2_wb.push(wb);
                continue;
            };
            let retired = retire_access(&wb, access, &mut self.vec_mem_inflight, &mut self.rob);
            let vlen_bits = state.hart.regs.vpr().vlen().bits();
            for value in retired.writes {
                let elems_per_reg = (vlen_bits / (value.eew.bytes() * 8)).max(1);
                let write = ElementWrite {
                    reg: VRegIdx::new(value.vd_phys.as_u16() as u8),
                    index: ElemIdx::new(value.elem_idx.as_usize() % elems_per_reg),
                    eew: value.eew,
                    data: value.value,
                };
                self.rob.push_vec_element_write(wb.rob_tag, write);
            }
            if retired.completed {
                self.vec_mem_inflight.retain(|m| m.rob_tag != wb.rob_tag);
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
            cycle: 0,
            redirect_latency: config.pipeline.redirect_latency(),
        }
    }
}

impl ExecutionEngine for InOrderEngine {
    fn tick(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<RenameIssueEntry>,
        redirect: &mut Option<u64>,
    ) {
        self.cycle += 1;
        let now = self.cycle;

        if let Some(squash) = self.common.take_due_squash(now) {
            self.apply_squash(state, squash, redirect);
            rename_output.clear();
        }

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

        match commit_event {
            Some(CommitEvent::Trap(trap, pc)) => {
                self.flush(state);
                state.trap(&trap, pc);
                *redirect = Some(state.hart.pc);
                return;
            }
            Some(CommitEvent::ReExecute(pc) | CommitEvent::SquashAfter(pc)) => {
                self.flush(state);
                state.hart.pc = pc;
                *redirect = Some(pc);
                return;
            }
            None => {}
        }

        self.retire_vec_mem_elements(state);
        writeback::writeback_stage(&state.stage(), &mut self.mem2_wb, &mut self.rob);

        let _ = memory2::memory2_stage(
            &state.stage(),
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            None,
            Some(&mut self.vec_store_buffer),
        );

        // Memory1 consumes execute_mem1, emits MemReq packets for misses,
        // and parks parked loads into self.common.outstanding_loads. SB
        // forwards and stores resolve straight into mem1_mem2.
        let mut input = std::mem::take(&mut self.execute_mem1);
        let resolved = memory1::memory1_stage(&mut state.stage(), self, &mut input);
        // Ops behind an unresolved translation walk go back; ops waiting on
        // a store-buffer drain live in `common.mem1_replay`.
        self.execute_mem1.extend(input);
        // A load forwarded at zero latency skips memory2 and writes back next cycle.
        self.mem2_wb.append(&mut self.common.forwarded_results);
        for span in resolved.expanded_spans {
            let _ = expand_span(&span, &mut self.vec_mem_inflight);
        }

        // Skip issue+execute when M1 hasn't drained, so we don't overwrite held entries.
        let backpressured = !self.execute_mem1.is_empty();

        self.deliver_ready_results(state, now);

        let (results, units) = if backpressured {
            (Vec::new(), Vec::new())
        } else {
            let (issued, units) = self.issuer.select(
                self.issue_width,
                &self.rob,
                &self.store_buffer,
                &self.vec_store_buffer,
                &mut state.stage(),
                &mut self.fu_pool,
                now,
                self.common.pending_squash,
            );
            if issued.is_empty() && !self.issuer.is_empty() {
                state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_data).inc();
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
        };
        if results.iter().any(|r| r.ctrl.vec_op.is_config()) {
            self.common.vector_config_unresolved = false;
        }
        self.hold_results(results, &units, now);
        self.issue_vec_mem_elements(state);

        // Dispatch even during backpressure: skipping it lets rename_output outgrow issue capacity.
        let rename_entries = std::mem::take(rename_output);
        if !rename_entries.is_empty() {
            self.issuer.dispatch(rename_entries);
        }
    }

    fn can_accept(&self) -> usize {
        // A store's buffer slot is checked per instruction, at rename.
        let rob_free = self.rob.free_slots();
        let issue_free = self.issuer.available_slots();
        rob_free.min(issue_free).min(self.rename_width)
    }

    fn flush(&mut self, state: &mut CoreCtx<'_>) {
        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.vec_store_buffer.flush_speculative();
        self.vec_mem_inflight.clear();
        self.scoreboard.flush();
        self.issuer.flush();
        self.pending.clear();
        self.execute_mem1.clear();
        self.common.mem1_replay.clear();
        self.common.pending_squash = None;
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
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

    fn has_register_renaming(&self) -> bool {
        false
    }

    /// Minor's fetch follows a branch the cycle it arrives.
    fn fetch_squashes_for_a_cycle(&self) -> bool {
        false
    }

    fn is_recovering_from_squash(&self) -> bool {
        false
    }

    fn load_queue_mut(
        &mut self,
    ) -> Option<&mut crate::uarch::pipeline::lsq::load_queue::LoadQueue> {
        None
    }

    fn rename(
        &mut self,
        state: &mut crate::uarch::ctx::StageCtx<'_>,
        id: crate::uarch::pipeline::latches::IdExEntry,
    ) -> crate::uarch::pipeline::engine::Renamed {
        self.rename_one(state, id)
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
        let config = Config::default();
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        assert_eq!(engine.can_accept(), config.pipeline.width);
    }

    #[test]
    fn test_inorder_engine_flush() {
        let config = Config::default();
        let mut engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        engine.flush(&mut state);

        assert_eq!(engine.execute_mem1.len(), 0);
        assert_eq!(engine.mem1_mem2.len(), 0);
        assert_eq!(engine.mem2_wb.len(), 0);
    }
}
