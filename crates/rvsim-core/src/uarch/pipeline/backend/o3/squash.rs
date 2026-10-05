//! Squashes: flushing after a trap or re-execution at commit, taking a
//! pending squash, and squashing a load that read memory too early.

use crate::isa::rvv::VRegIdx;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::backend::shared::commit::{
    self, CommitEvent, CommitRegisters, CommitResources,
};
use crate::uarch::pipeline::backend::shared::flush_stats::count_flush;
use crate::uarch::pipeline::engine::ExecutionEngine;
use crate::uarch::pipeline::rob::RobTag;
use crate::uarch::pipeline::squash::{PendingSquash, Redirect, SquashCause};

use super::O3Engine;

/// A load that read a location before an older store wrote it, and that
/// store's PC.
pub(super) type OrderViolation = (RobTag, u64);

/// The older of the violations memory2 and memory1 found.
pub(super) const fn older_violation(
    memory2: Option<OrderViolation>,
    memory1: Option<OrderViolation>,
) -> Option<OrderViolation> {
    match (memory2, memory1) {
        (Some(m2), Some(m1)) if m1.0.is_older_than(m2.0) => Some(m1),
        (Some(m2), _) => Some(m2),
        (None, m1) => m1,
    }
}

impl O3Engine {
    /// Counts a cycle commit spends squashing the ROB, during which rename
    /// is blocked.
    pub(super) fn count_squash_stall(&mut self, state: &mut CoreCtx<'_>) {
        if self.squash_stall_remaining > 0 {
            self.squash_stall_remaining -= 1;
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_squash).inc();
        }
    }

    /// Cycles rename waits after a squash is taken. Commit squashes the ROB
    /// at `squash_width` entries per cycle (gem5's `ROB::doSquash`), at
    /// least one cycle; dispatch holds while it sees commit squashing and
    /// rename while it sees dispatch held, each a cycle late, so rename
    /// resumes one cycle after the squash finishes. The rename map itself
    /// is restored at once, as gem5 undoes its history buffer.
    pub(super) fn squash_cycles(&self, squashed: usize) -> u64 {
        squashed.div_ceil(self.squash_width.max(1)).max(1) as u64 + 1
    }

    /// Retires what the ROB head allows. A trap or a re-execution at commit
    /// flushes the whole window and redirects fetch; returns whether it did.
    pub(super) fn retire(&mut self, state: &mut CoreCtx<'_>, redirect: &mut Option<u64>) -> bool {
        let commit_event = commit::commit_stage(
            state,
            CommitResources {
                common: &mut self.common,
                rob: &mut self.rob,
                store_buffer: &mut self.store_buffer,
                vec_store_buffer: &mut self.vec_store_buffer,
                width: self.commit_width,
                registers: CommitRegisters::Renamed {
                    rename_map: &mut self.committed_rename_map,
                    free_list: &mut self.free_list,
                    load_queue: &mut self.load_queue,
                    checkpoints: &mut self.checkpoints,
                    vec_prf: &mut self.vec_prf,
                    vec_free_list: &mut self.vec_free_list,
                },
            },
        );

        if let Some(event) = &commit_event {
            count_flush(state, event.into(), self.rob.len());
        }
        match commit_event {
            Some(CommitEvent::Trap(trap, pc)) => {
                let squashed = self.rob.len();
                self.flush(state);
                self.squash_stall_remaining = self.squash_cycles(squashed);
                state.trap(&trap, pc);
                *redirect = Some(state.hart.pc);
                true
            }
            Some(CommitEvent::ReExecute(pc) | CommitEvent::SquashAfter(pc)) => {
                let squashed = self.rob.len();
                self.flush(state);
                self.squash_stall_remaining = self.squash_cycles(squashed);
                state.hart.pc = pc;
                *redirect = Some(pc);
                true
            }
            None => false,
        }
    }

    /// Squashes from the load of a memory-order violation, or from the
    /// oldest load a coherence invalidation caught, whichever is older;
    /// the load re-executes, so it does not survive either.
    pub(super) fn squash_on_violation(
        &mut self,
        state: &mut CoreCtx<'_>,
        now: u64,
        order_violation: Option<OrderViolation>,
    ) {
        let squash = match (order_violation, self.common.coherence_violation.take()) {
            (Some((tag, _)), Some(coherence_tag)) if coherence_tag.is_older_than(tag) => {
                Some((coherence_tag, None))
            }
            (Some((tag, store_pc)), _) => Some((tag, Some(store_pc))),
            (None, Some(coherence_tag)) => Some((coherence_tag, None)),
            (None, None) => None,
        };
        let Some((violating_tag, store_pc)) = squash else { return };

        let violation_pc = self.rob.find_entry(violating_tag).map_or(state.hart.pc, |e| e.pc);
        let cause = if let Some(store_pc) = store_pc {
            self.mdp.violation(violation_pc, store_pc);
            SquashCause::MemoryOrder
        } else {
            state.uncore.stats.counter(state.core.stat_paths.lsq.coherence_violations).inc();
            SquashCause::Coherence
        };
        self.common.request_squash(PendingSquash {
            keep_tag: self.rob.prev_tag_of(violating_tag),
            redirect: Redirect::to(violation_pc, cause),
            apply_at: now + self.redirect_latency,
        });
    }

    /// Rebuild the speculative rename map after a partial flush by replaying surviving ROB entries.
    fn rebuild_rename_map(&mut self) {
        self.rename_map = self.committed_rename_map.clone();
        for entry in self.rob.iter_in_order() {
            if entry.ctrl.reg_write && !entry.rd.is_zero() {
                self.rename_map.set(entry.rd, false, entry.phys_dst);
            } else if entry.ctrl.fp_reg_write {
                self.rename_map.set(entry.rd, true, entry.phys_dst);
            }
            if entry.vec_dst_count > 0 {
                let vd_base = entry.ctrl.vd.as_u8();
                for i in 0..entry.vec_dst_count as usize {
                    let vreg = VRegIdx::new(vd_base + i as u8);
                    self.rename_map.set_vec(vreg, entry.vec_phys_dst[i]);
                }
            }
        }
    }

    /// Takes a squash: drops everything younger than its kept tag (or the
    /// whole window when that tag has already retired), reclaims their
    /// physical registers, restores the rename map, and redirects fetch.
    pub(super) fn apply_squash(
        &mut self,
        state: &mut CoreCtx<'_>,
        squash: PendingSquash,
        redirect: &mut Option<u64>,
    ) {
        self.serialization.squash(|tag| squash.squashes(tag));
        let keep_tag = squash.keep_tag.filter(|tag| self.rob.find_entry(*tag).is_some());
        let keep_seq = keep_tag.and_then(|tag| self.rob.find_entry(tag)).map(|entry| entry.seq);
        let squashed = if let Some(keep_tag) = keep_tag {
            for entry in self.rob.iter_after(keep_tag) {
                self.free_list.reclaim(entry.phys_dst);
                for i in 0..entry.vec_dst_count as usize {
                    self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                }
            }
            self.rob.iter_after(keep_tag).count()
        } else {
            for entry in self.rob.iter_all() {
                self.free_list.reclaim(entry.phys_dst);
                for i in 0..entry.vec_dst_count as usize {
                    self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                }
            }
            self.rob.len()
        };
        count_flush(state, squash.redirect.cause.into(), squashed);

        if let Some(keep_tag) = keep_tag {
            // flush_after, not flush: older un-issued IQ entries must survive or deadlock the pipeline.
            self.issue_queue.flush_after(keep_tag);
            self.rob.flush_after(keep_tag);
            self.store_buffer.flush_after(keep_tag);
            self.load_queue.flush_after(keep_tag);
            self.mdp.flush_after(keep_tag, &self.rob);
            self.vec_store_buffer.flush_after(keep_tag);
            self.common.squash_after(keep_tag);
        } else {
            self.issue_queue.flush();
            self.rob.flush_all();
            self.store_buffer.flush_speculative();
            self.load_queue.flush();
            self.mdp.flush();
            self.vec_store_buffer.flush_speculative();
            self.common.squash_all();
        }
        let survives = |tag: RobTag| keep_tag.is_some_and(|keep_tag| tag.is_older_or_eq(keep_tag));
        self.mem1_mem2.retain(|e| survives(e.rob_tag));
        self.mem2_wb.retain(|e| survives(e.rob_tag));
        self.pending_results.retain(|p| survives(p.entry.rob_tag));
        self.pending_addresses.retain(|p| survives(p.entry.rob_tag));
        self.vec_pending.retain(|v| survives(v.rob_tag));
        self.vec_mem_pending.retain(|m| survives(m.entry.rob_tag));
        self.vec_mem_inflight.retain(|m| survives(m.rob_tag));
        self.execute_mem1.retain(|e| survives(e.rob_tag));

        let checkpoint = keep_tag
            .filter(|_| self.checkpoints.capacity() > 0)
            .and_then(|tag| self.checkpoints.find_by_tag(tag).map(|ckpt| ckpt.rename_map.clone()));
        if let Some(rename_map) = checkpoint {
            self.rename_map = rename_map;
        } else {
            self.rebuild_rename_map();
        }
        self.squash_stall_remaining = self.squash_cycles(squashed);
        if let Some(keep_tag) = keep_tag {
            self.checkpoints.flush_after(keep_tag);
        } else {
            self.checkpoints.flush_all();
        }

        *redirect = Some(squash.redirect.target);
        let now = state.cycle;
        self.common.squash_predictions(&mut state.core.branch_predictor, &squash, keep_seq, now);
    }
}
