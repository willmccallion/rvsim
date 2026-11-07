//! Completing results the functional units have finished: writing them
//! back, completing their ROB entries and waking their dependents.

use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::rename::vec_prf::{VecPhysReg, VecPhysRegFile};
use crate::uarch::pipeline::trace::Stage;

use super::issue_queue::IssueQueue;
use super::{O3Engine, PendingResult};

/// Marks a vector instruction's destination registers ready, then wakes
/// the instructions waiting on them.
pub(super) fn wake_vector_dests(
    vec_prf: &mut VecPhysRegFile,
    issue_queue: &mut IssueQueue,
    dests: &[VecPhysReg],
) {
    for &reg in dests {
        vec_prf.mark_ready(reg);
    }
    for &reg in dests {
        issue_queue.wakeup_vec_phys(reg, vec_prf);
    }
}

impl O3Engine {
    /// Takes up to `slots` of the results finished by `now`, earliest
    /// finished and then oldest first; the rest wait for later slots.
    fn take_finished_results(&mut self, now: u64, slots: usize) -> Vec<PendingResult> {
        let (mut finished, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_results)
            .into_iter()
            .partition(|p| p.complete_cycle <= now);
        finished.sort_by(|a, b| {
            a.complete_cycle
                .cmp(&b.complete_cycle)
                .then_with(|| a.entry.op.rob_tag.age_cmp(b.entry.op.rob_tag))
        });
        self.pending_results = waiting;
        self.pending_results.extend(finished.split_off(slots.min(finished.len())));
        finished
    }

    /// Writes back up to `slots` of the non-memory results finished by `now`:
    /// completes their ROB entries, or records their faults, and wakes their
    /// dependents. Returns how many slots it used.
    pub(super) fn writeback_finished_results(
        &mut self,
        state: &mut CoreCtx<'_>,
        now: u64,
        slots: usize,
    ) -> usize {
        let finished = self.take_finished_results(now, slots);
        let used = finished.len();
        for PendingResult { entry, .. } in finished {
            if let Some(seq) = self.rob.find_entry(entry.op.rob_tag).map(|e| e.seq) {
                state.trace_stage(seq, Stage::Writeback);
            }
            if let Some(trap) = entry.op.trap {
                self.rob.fault(entry.op.rob_tag, trap);
                continue;
            }
            let val = entry.register_value();
            if !entry.op.fp_flags.is_empty() {
                self.rob.set_fp_flags(entry.op.rob_tag, entry.op.fp_flags);
            }
            if let Some(info) = entry.op.sfence_vma {
                self.rob.set_sfence_vma(entry.op.rob_tag, info);
            }
            // CSR writes deferred to commit so speculative state isn't observed on trap.
            self.rob.complete(entry.op.rob_tag, val);
            self.prf.write(entry.op.rd_phys, val);
            self.issue_queue.wakeup_phys(entry.op.rd_phys, val);
        }
        used
    }

    /// Advances in-flight vector arithmetic: wakes dependents once the first
    /// element group is ready (chaining), and completes the instruction in
    /// one of the remaining `slots` once all its groups are.
    pub(super) fn complete_vector_results(&mut self, now: u64, mut slots: usize) {
        let mut i = 0;
        while i < self.vec_pending.len() {
            let vp = &mut self.vec_pending[i];
            if vp.should_wakeup(now) {
                wake_vector_dests(&mut self.vec_prf, &mut self.issue_queue, vp.vd.as_slice());
                vp.wakeup_fired = true;
            }
            // Some vl=0 ops reach full_complete before first_group_ready; wake here too.
            if vp.is_complete(now) && slots > 0 {
                slots -= 1;
                if !vp.wakeup_fired {
                    wake_vector_dests(&mut self.vec_prf, &mut self.issue_queue, vp.vd.as_slice());
                    vp.wakeup_fired = true;
                }
                self.rob.complete(vp.rob_tag, 0);
                let _ = self.vec_pending.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}
