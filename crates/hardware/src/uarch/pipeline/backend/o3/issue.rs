//! Dispatch into the issue queue, and select and issue out of it.

use crate::exec::compute::vector::execute::execute_vec_op_on;
use crate::exec::compute::vector::mem::{
    check_vec_mem_emul, element_accesses, is_vec_store, vec_mem_dst_count,
};
use crate::isa::rvv::{VRegIdx, parse_vtype};
use crate::system::CoreCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::{
    VecMemInflight, micro_ops_for, moves_in_spans, plan_accesses, route_to_phys,
};
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::uarch::pipeline::rename::vec_prf::{VecPhysReg, VecPrfView};
use crate::uarch::pipeline::squash::PendingSquash;
use crate::uarch::vector::chaining::VecPendingResult;
use crate::uarch::vector::lane_model;

use super::execute;
use super::fu_pool::FuType;
use super::issue_queue::{IssueBudget, SelectedEntry};
use super::{O3Engine, PendingResult};

/// A vector instruction's destination register group.
#[derive(Clone, Copy, Debug)]
struct VecDest {
    /// Its renamed physical registers.
    phys: [VecPhysReg; 8],
    /// How many registers the group has.
    count: u8,
    /// The first architectural register of the group.
    reg: VRegIdx,
}

/// A vector instruction that has issued, on its way into the vector unit.
struct IssuedVector<'a> {
    entry: &'a RenameIssueEntry,
    result: ExMem1Entry,
    fu_type: FuType,
    dest: Option<VecDest>,
    complete_cycle: u64,
}

/// Points `count` architectural registers from `base` at `regs`.
fn overlay(mapping: &mut [VecPhysReg; 32], base: VRegIdx, regs: &[VecPhysReg; 8], count: u8) {
    let base = base.as_u8() as usize;
    for i in 0..count as usize {
        if base + i < 32 {
            mapping[base + i] = regs[i];
        }
    }
}

impl O3Engine {
    /// Selects ready instructions within the issue width, ports and free
    /// units, and issues them.
    pub(super) fn issue(&mut self, state: &mut CoreCtx<'_>, now: u64, memory_blocked: bool) {
        let budget = IssueBudget {
            width: self.issue_width,
            load_ports: self.load_ports,
            store_ports: self.store_ports,
            units: &self.fu_pool,
            now,
            memory_blocked,
        };
        let selection = self.issue_queue.select(&budget, &self.store_buffer, &self.rob);
        let stalled_fu = selection.unit_stalls > 0;
        if stalled_fu {
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_fu_structural).inc();
        }

        let issued_any = !selection.entries.is_empty();
        for selected in selection.entries {
            self.issue_one(state, selected, now);
        }

        if !issued_any && !stalled_fu && !self.issue_queue.is_empty() {
            state.uncore.stats.counter(state.core.stat_paths.pipeline.stalls_data).inc();
        }
    }

    /// Issues one selected instruction: reserves its unit, executes it, and
    /// tracks its result until the unit is done with it.
    fn issue_one(&mut self, state: &mut CoreCtx<'_>, selected: SelectedEntry, now: u64) {
        let SelectedEntry { entry, fu_type, unit } = selected;
        if entry.inst.ctrl.mem_read || entry.inst.ctrl.uses_store_buffer() {
            self.mdp.issued(entry.rob_tag);
        }

        // vsetvl* run synchronously in execute_one; exclude from deferred VecPrfView.
        let is_vec_config = entry.inst.ctrl.vec_op.is_config();
        let is_vec_arith = fu_type.is_vector() && fu_type != FuType::VecMem && !is_vec_config;
        let is_vec_mem = fu_type == FuType::VecMem;
        let dest = vector_dest(&entry, is_vec_mem);

        let complete_cycle = if is_vec_arith {
            let latency = self.fu_pool.vector_op_latency(
                fu_type,
                &entry.inst.ctrl,
                entry.vec_vl as usize,
                self.num_vec_lanes.as_usize(),
            );
            self.fu_pool.acquire_with_latency(unit, now, latency)
        } else {
            // A vector memory op's unit is the address generator;
            // its elements pay their latency in memory1 and memory2.
            self.fu_pool.acquire(unit, now)
        };

        let (result, redirect) = execute::execute_one(&mut state.stage(), &entry, &mut self.rob);
        if let Some(redirect) = redirect {
            self.common.request_squash(PendingSquash {
                keep_tag: Some(result.rob_tag),
                redirect,
                apply_at: complete_cycle + self.redirect_latency,
            });
        }
        if is_vec_config {
            self.common.vector_config_unresolved = false;
        }

        if (is_vec_arith || is_vec_mem) && result.trap.is_none() {
            let issued = IssuedVector { entry: &entry, result, fu_type, dest, complete_cycle };
            if is_vec_arith {
                self.execute_vector_arith(state, issued, now);
            } else {
                self.start_vector_memory(state, issued, now);
            }
            return;
        }

        let pending = PendingResult { entry: result, complete_cycle, fu_type };
        if pending.entry.ctrl.uses_memory_pipeline() {
            self.pending_addresses.push(pending);
        } else {
            self.pending_results.push(pending);
        }
    }

    /// The speculative rename map's vector mappings, one per architectural
    /// register.
    fn vec_rename_view(&self) -> [VecPhysReg; 32] {
        std::array::from_fn(|i| self.rename_map.get_vec(VRegIdx::new(i as u8)))
    }

    /// Runs a vector arithmetic instruction on the registers it renamed,
    /// with the vtype and vl it was dispatched under, and tracks its result
    /// until its lanes finish.
    fn execute_vector_arith(&mut self, state: &CoreCtx<'_>, issued: IssuedVector<'_>, now: u64) {
        let IssuedVector { entry, result, fu_type, dest, complete_cycle } = issued;
        let ctrl = &entry.inst.ctrl;

        // Map through the rename-time registers so later renames don't alias.
        let mut mapping = self.vec_rename_view();
        overlay(&mut mapping, ctrl.vs2, &entry.vs2_phys, entry.vec_src2_count);
        overlay(&mut mapping, ctrl.vs1, &entry.vs1_phys, entry.vec_src1_count);
        if let Some(dest) = dest {
            let base = dest.reg.as_u8() as usize;
            for i in 0..dest.count as usize {
                if base + i < 32 {
                    // Pre-copy old vd so tail/mask-undisturbed reads see correct baseline.
                    if i < entry.vec_src3_count as usize {
                        self.vec_prf.copy_reg(dest.phys[i], entry.vs3_phys[i]);
                    }
                    mapping[base + i] = dest.phys[i];
                }
            }
        }
        if !entry.mask_phys.is_zero() {
            mapping[0] = entry.mask_phys;
        }

        let executed = execute_vec_op_on(
            &mut VecPrfView::new(&mut self.vec_prf, mapping),
            entry.vec_vtype,
            entry.vec_vl,
            entry.vec_vstart,
            entry.vec_vxrm,
            entry.vec_frm,
            state.config.isa.vector.elen,
            state.config.isa.vector.zvfh,
            &entry.inst,
        );
        let vec_result = match executed {
            Ok(r) => r,
            Err(trap) => {
                self.rob.fault(result.rob_tag, trap, ExceptionStage::Execute);
                return;
            }
        };
        if !vec_result.fp_flags.is_empty() {
            self.rob.set_fp_flags(result.rob_tag, vec_result.fp_flags.bits());
        }
        if vec_result.vxsat {
            self.rob.set_vxsat(result.rob_tag, true);
        }

        let first_ready = lane_model::first_group_ready(now, self.fu_pool.startup_latency(fu_type));
        if result.ctrl.vec_reg_write {
            self.vec_pending.push(VecPendingResult {
                rob_tag: result.rob_tag,
                vd_phys: dest.map_or([VecPhysReg::ZERO; 8], |d| d.phys),
                vd_count: dest.map_or(0, |d| d.count),
                first_group_ready: first_ready,
                full_complete: complete_cycle,
                wakeup_fired: false,
            });
        } else {
            // Scalar-result ops (vmv.x.s, vcpop.m, vfirst.m) take the scalar path.
            let mut scalar = result;
            scalar.alu = vec_result.scalar_result.unwrap_or(0);
            self.pending_results.push(PendingResult { entry: scalar, complete_cycle, fu_type });
        }
    }

    /// Generates a vector memory instruction's element accesses and plans
    /// them into the micro-ops the memory stages will carry, released in
    /// waves as the load queue has room.
    fn start_vector_memory(&mut self, state: &CoreCtx<'_>, issued: IssuedVector<'_>, now: u64) {
        let IssuedVector { entry, result, fu_type, dest, complete_cycle } = issued;
        let ctrl = &entry.inst.ctrl;
        let vec_op = result.ctrl.vec_op;
        let is_store = is_vec_store(vec_op);
        let vd_count = dest.map_or(0, |d| d.count);
        let vd_phys = dest.map_or([VecPhysReg::ZERO; 8], |d| d.phys);

        // Reject illegal EMUL (>8) before element_accesses would panic.
        let vtype = parse_vtype(entry.vec_vtype);
        if let Err(trap) = check_vec_mem_emul(result.inst, vec_op, ctrl, &vtype) {
            self.rob.fault(result.rob_tag, trap, ExceptionStage::Execute);
            return;
        }

        let mut mapping = self.vec_rename_view();
        overlay(&mut mapping, ctrl.vs2, &entry.vs2_phys, entry.vec_src2_count);
        overlay(&mut mapping, ctrl.vd, &entry.vs3_phys, entry.vec_src3_count);
        if !entry.mask_phys.is_zero() {
            mapping[0] = entry.mask_phys;
        }
        let micro_ops = route_to_phys(
            element_accesses(
                &VecPrfView::new(&mut self.vec_prf, mapping),
                result.alu,
                result.store_data as i64,
                ctrl,
                entry.vec_vtype,
                entry.vec_vl as usize,
                entry.vec_vstart as usize,
                vec_op,
            ),
            &vd_phys,
            vd_count,
        );

        // Pre-copy old vd so tail / mask-undisturbed elements observe prior values.
        if !is_store && let Some(dest) = dest {
            let copy_count = (dest.count as usize).min(entry.vec_src3_count as usize);
            for (i, &dst) in dest.phys.iter().enumerate().take(copy_count) {
                self.vec_prf.copy_reg(dst, entry.vs3_phys[i]);
            }
        }

        if is_store {
            self.vec_store_buffer.set_expected_elements(result.rob_tag, micro_ops.len());
        }
        if micro_ops.is_empty() {
            // VL=0 / vill=1: route through vec_pending so destination physregs surface ready.
            let first_ready =
                lane_model::first_group_ready(now, self.fu_pool.startup_latency(fu_type));
            self.vec_pending.push(VecPendingResult {
                rob_tag: result.rob_tag,
                vd_phys,
                vd_count,
                first_group_ready: first_ready,
                full_complete: complete_cycle,
                wakeup_fired: false,
            });
            return;
        }
        let width = state.config.pipeline.vector_mem_width_bytes();
        let planned = plan_accesses(micro_ops, moves_in_spans(vec_op), width);
        let all_micro_ops = micro_ops_for(&result, planned, is_store);
        self.vec_mem_inflight.push(VecMemInflight {
            rob_tag: result.rob_tag,
            remaining: all_micro_ops.len(),
            vd_phys,
            vd_count,
            wakeup_fired: false,
            pending_micro_ops: all_micro_ops,
            trimmed_at: None,
            fault: None,
        });
    }

    /// Dispatches renamed instructions into the issue queue, each with the
    /// memory dependence the MDP predicts for it.
    pub(super) fn dispatch(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<RenameIssueEntry>,
    ) {
        for entry in std::mem::take(rename_output) {
            let is_load = entry.inst.ctrl.mem_read;
            let is_store = entry.inst.ctrl.uses_store_buffer();
            let is_atomic = entry.inst.ctrl.atomic_op.is_some();
            let mem_dep =
                self.mdp.dispatch(entry.inst.pc, entry.rob_tag, is_load, is_store, is_atomic);
            let ok = self.issue_queue.dispatch(
                entry,
                &self.rob,
                &state.stage(),
                Some(&self.prf),
                Some(&self.vec_prf),
                mem_dep,
            );
            debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
        }
    }

    /// Mirrors the memory dependence unit's running totals into the stats.
    pub(super) fn publish_mdp_stats(&self, state: &mut CoreCtx<'_>) {
        let totals = self.mdp.stats();
        let paths = &state.core.stat_paths.mdp;
        for (stat, value) in [
            (paths.predictions_bypass, totals.predictions_bypass),
            (paths.predictions_wait_all, totals.predictions_wait_all),
            (paths.predictions_wait_for, totals.predictions_wait_for),
            (paths.violations, totals.violations),
        ] {
            let counter = state.uncore.stats.counter(stat);
            counter.reset();
            counter.add(value);
        }
    }
}

/// The destination register group of a vector instruction that writes
/// vector registers; a vector memory instruction's group is its
/// `nf × EMUL` data registers.
fn vector_dest(entry: &RenameIssueEntry, is_vec_mem: bool) -> Option<VecDest> {
    let ctrl = &entry.inst.ctrl;
    let mut groups = ctrl.vec_op.operand_groups(
        ctrl.vec_lmul_regs,
        ctrl.vec_lmul_is_fractional,
        ctrl.vec_src_encoding,
        ctrl.vec_nf,
        ctrl.vec_broadcast_vs2,
    );
    if is_vec_mem {
        let vtype = parse_vtype(entry.vec_vtype);
        if !vtype.vill {
            groups.vd =
                vec_mem_dst_count(ctrl.vec_op, ctrl.vec_eew, vtype.vsew, vtype.vlmul, ctrl.vec_nf);
        }
    }
    (ctrl.vec_reg_write && groups.vd > 0).then_some(VecDest {
        phys: entry.vd_phys,
        count: groups.vd,
        reg: ctrl.vd,
    })
}
