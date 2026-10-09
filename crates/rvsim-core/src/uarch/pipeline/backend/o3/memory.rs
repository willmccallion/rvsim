//! The memory stages as the out-of-order engine drives them: memory1 and
//! memory2, the writeback of their results, and the vector memory
//! micro-ops that flow through them.

use crate::exec::signals::ControlFlow;
use crate::isa::rvv::ElemIdx;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::{expand_span, retire_access};
use crate::uarch::pipeline::backend::shared::{memory1, memory2, writeback};
use crate::uarch::pipeline::latches::Mem2WbEntry;

use super::O3Engine;
use super::complete::wake_vector_dests;
use super::squash::OrderViolation;

impl O3Engine {
    /// Memory2: finalizes loads, which write back in the same cycle as
    /// gem5's LSQ hands a returned load to the current cycle's writeback,
    /// and wakes the loads that waited on the stores it resolves (SCs, AMOs
    /// and vector stores). Returns the violation it found, if any.
    pub(super) fn memory2(&mut self, state: &mut CoreCtx<'_>) -> Option<OrderViolation> {
        let mut memory2_results = Vec::with_capacity(self.mem1_mem2.len());
        let violation = memory2::memory2_stage(
            &state.stage(),
            &mut self.mem1_mem2,
            &mut memory2_results,
            &mut self.store_buffer,
            Some(&mut self.load_queue),
            Some(&mut self.vec_store_buffer),
        );
        for entry in &memory2_results {
            if entry.ctrl.mem_write
                && (entry.ctrl.atomic_op.is_some() || entry.vec_mem.is_some())
                && let Some(store_tag) = self.mdp.store_resolved(entry.rob_tag)
            {
                self.issue_queue.wakeup_mem_dep(&[store_tag]);
            }
        }
        self.mem2_wb.extend(memory2_results);
        violation
    }

    /// Takes the vector memory micro-ops out of the writeback latch: writes
    /// their elements to the vector register file and, once an instruction's
    /// last element is back, wakes its dependents.
    pub(super) fn retire_vector_memory_results(&mut self) {
        let mut scalar_wb = Vec::with_capacity(self.mem2_wb.len());
        for wb in std::mem::take(&mut self.mem2_wb) {
            let Some(ref vme) = wb.vec_mem else {
                scalar_wb.push(wb);
                continue;
            };
            if !vme.is_store {
                self.load_queue.mark_written_back(wb.rob_tag, vme.micro_op);
            }
            let retired = retire_access(&wb, vme, &mut self.vec_mem_inflight, &mut self.rob);
            let vlen_bits = self.vec_prf.vlen().bits();
            for write in retired.writes {
                let elems_per_reg = (vlen_bits / (write.eew.bytes() * 8)).max(1);
                let local = ElemIdx::new(write.elem_idx.as_usize() % elems_per_reg);
                self.vec_prf.write_element(write.vd_phys, local, write.eew, write.value);
            }
            // Dependents bulk-read all elements, so they wake on full completion only.
            if retired.completed
                && let Some(parent) =
                    self.vec_mem_inflight.iter_mut().find(|m| m.rob_tag == wb.rob_tag)
                && !parent.wakeup_fired
            {
                wake_vector_dests(
                    &mut self.vec_prf,
                    &mut self.issue_queue,
                    &parent.vd_phys[..parent.vd_count as usize],
                );
                parent.wakeup_fired = true;
            }
        }
        self.mem2_wb = scalar_wb;
    }

    /// Writes back as many memory results as there are writeback slots,
    /// waking their dependents; the rest wait for a later cycle. Returns the
    /// slots left.
    pub(super) fn writeback_memory_results(&mut self, state: &mut CoreCtx<'_>) -> usize {
        let later = self.mem2_wb.split_off(self.mem2_wb.len().min(self.writeback_width));
        let now = std::mem::replace(&mut self.mem2_wb, later);
        let used = now.len();
        self.write_back(state, now);
        self.writeback_width - used
    }

    /// Writes back the loads memory1 forwarded this cycle at zero latency,
    /// as many as `slots` allow; the rest wait with the memory results for
    /// a later cycle. Returns how many slots it used.
    pub(super) fn writeback_forwarded_loads(
        &mut self,
        state: &mut CoreCtx<'_>,
        slots: usize,
    ) -> usize {
        let mut now = std::mem::take(&mut self.common.forwarded_results);
        let mut later = now.split_off(slots.min(now.len()));
        let used = now.len();
        self.write_back(state, now);
        if !later.is_empty() {
            later.append(&mut self.mem2_wb);
            self.mem2_wb = later;
        }
        used
    }

    /// Completes `entries` in the ROB and wakes their dependents.
    fn write_back(&mut self, state: &mut CoreCtx<'_>, mut entries: Vec<Mem2WbEntry>) {
        // Taken before writeback consumes the entries, so dependents wake via the PRF.
        let wakeups: Vec<_> = entries
            .iter()
            .filter(|wb| wb.trap.is_none())
            .map(|wb| {
                let val = if wb.ctrl.mem_read {
                    wb.load_data
                } else if wb.ctrl.control_flow == ControlFlow::Jump {
                    wb.pc.wrapping_add(wb.inst_size.as_u64())
                } else {
                    wb.alu
                };
                (wb.rd_phys, val)
            })
            .collect();

        writeback::writeback_stage(&state.stage(), &mut entries, &mut self.rob);

        for (rd_phys, val) in wakeups {
            self.prf.write(rd_phys, val);
            self.issue_queue.wakeup_phys(rd_phys, val);
        }
    }

    /// Memory1: translates and starts the accesses whose addresses are ready.
    /// It always accepts work, parking loads in `common.outstanding_loads`;
    /// backpressure comes from the L1D's pending table and surfaces as
    /// mailbox-drain backlogs. Returns the violation it found, if any.
    pub(super) fn memory1(&mut self, state: &mut CoreCtx<'_>, now: u64) -> Option<OrderViolation> {
        self.send_generated_addresses_to_memory1(now);
        let mut input = std::mem::take(&mut self.execute_mem1);
        let resolved = memory1::memory1_stage(&mut state.stage(), self, &mut input);
        self.execute_mem1.extend(input);
        for span in resolved.expanded_spans {
            let (rob_tag, is_load) = (span.rob_tag, span.ctrl.mem_read);
            if let Some(micro_op) = expand_span(&span, &mut self.vec_mem_inflight)
                && is_load
            {
                self.load_queue.unclaim(rob_tag, micro_op);
            }
        }
        for store_tag in resolved.resolved_stores {
            if let Some(tag) = self.mdp.store_resolved(store_tag) {
                self.issue_queue.wakeup_mem_dep(&[tag]);
            }
        }
        resolved.violation
    }

    /// Whether memory1 still holds ops behind an unresolved translation walk,
    /// which blocks memory issue this cycle. Ops waiting on a store-buffer
    /// drain live in `common.mem1_replay` and never gate issue.
    pub(super) fn note_memory_backpressure(&self, state: &mut CoreCtx<'_>) -> bool {
        let backpressured = !self.execute_mem1.is_empty();
        if backpressured {
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_backpressure).inc();
        }
        backpressured
    }

    /// Sends the memory ops whose address generation finishes this cycle to
    /// memory1, which runs later in the same cycle, as an in-order memory
    /// op issued last cycle reaches it.
    fn send_generated_addresses_to_memory1(&mut self, now: u64) {
        let (ready, waiting) = std::mem::take(&mut self.pending_addresses)
            .into_iter()
            .partition(|p| p.complete_cycle <= now);
        self.pending_addresses = waiting;
        for done in ready {
            self.execute_mem1.push(done.entry);
        }
    }

    /// Moves pending vector memory micro-ops into `vec_mem_pending`, each
    /// load micro-op claiming one of its instruction's load-queue slots. A
    /// load that has outgrown its slots goes on only as the oldest memory
    /// access in flight, reusing the slots of micro-ops that have written
    /// back, since no older store is left to resolve over them.
    pub(super) fn issue_vec_mem_waves(&mut self) {
        for inflight in &mut self.vec_mem_inflight {
            if inflight.outgrew_load_slots {
                if self.rob.has_older_memory_access(inflight.rob_tag) {
                    continue;
                }
                self.load_queue.unclaim_written_back(inflight.rob_tag);
            }
            while let Some(front) = inflight.pending_micro_ops.front() {
                if !front.is_store
                    && !self.load_queue.claim(front.entry.rob_tag, front.micro_op, front.bytes)
                {
                    inflight.outgrew_load_slots = true;
                    break;
                }
                let Some(mop) = inflight.pending_micro_ops.pop_front() else { break };
                self.vec_mem_pending.push_back(mop);
            }
        }
    }

    /// Sends pending vector memory micro-ops to memory1, up to the load and
    /// store ports, in order.
    pub(super) fn send_vector_memory_micro_ops(&mut self) {
        let mut loads_issued = 0usize;
        let mut stores_issued = 0usize;
        while let Some(front) = self.vec_mem_pending.front() {
            if front.is_store {
                if stores_issued >= self.store_ports {
                    break;
                }
                stores_issued += 1;
            } else {
                if loads_issued >= self.load_ports {
                    break;
                }
                loads_issued += 1;
            }
            let Some(mop) = self.vec_mem_pending.pop_front() else { break };
            self.execute_mem1.push(mop.entry);
        }
    }
}
