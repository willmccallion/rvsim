//! Rename Stage: ROB allocation, store buffer allocation, scoreboard/PRF marking.
//!
//! For the in-order backend: uses scoreboard to track register producers.
//! For the O3 backend (`has_prf` = true): uses physical register file, free list,
//! and rename map to implement full register renaming.
//!
//! Source register tags are captured BEFORE the scoreboard is updated for rd,
//! so that instructions reading their own destination (e.g. ADDI x5, x5, 16)
//! get the previous producer's tag, not their own.

use crate::sim::SimState;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::signals::{ControlFlow, VectorOp};
use crate::core::units::vpu::mem::{
    is_vec_load, is_vec_store, vec_mem_dst_count, vec_mem_emul_regs,
};
use crate::core::units::vpu::types::{VRegIdx, VecPhysReg, parse_vtype};
use crate::trace_rename;

/// Executes the rename stage: allocate ROB/SB entries, capture source tags, mark scoreboard.
///
/// # Panics
///
/// Panics if checkpoint allocation fails after the stall check indicated a slot was available.
pub fn rename_stage<E: ExecutionEngine>(
    state: &mut SimState,
    input: &mut Vec<IdExEntry>,
    engine: &mut E,
    rename_output: &mut Vec<RenameIssueEntry>,
) {
    let entries = std::mem::take(input);

    // Track a budget counter manually: can_accept() doesn't see rename_output growth.
    let mut budget = engine.can_accept();

    for id in entries {
        if budget == 0 {
            if input.is_empty() {
                state.stats.stalls_dispatch += 1;
            }
            input.push(id);
            continue;
        }

        if engine.has_prf() {
            let is_branch_or_jump =
                matches!(id.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump);
            if is_branch_or_jump
                && engine.checkpoint_count() > 0
                && engine.checkpoint_table().is_full()
            {
                state.stats.stalls_checkpoint += 1;
                // Set budget=0 so remaining iterations also push back to input.
                budget = 0;
                input.push(id);
                continue;
            }

            // Capture source physical regs BEFORE updating rename map for rd.
            let rs1_phys = engine.rename_map().get(id.rs1, id.ctrl.rs1_fp);
            let rs2_phys = engine.rename_map().get(id.rs2, id.ctrl.rs2_fp);
            let rs3_phys =
                if id.ctrl.rs3_fp { engine.rename_map().get(id.rs3, true) } else { PhysReg(0) };

            // Capture vector source mappings before vd is renamed.
            let lmul = id.ctrl.vec_lmul_regs;
            let mut grp = id.ctrl.vec_op.operand_groups(
                lmul,
                id.ctrl.vec_lmul_is_fractional,
                id.ctrl.vec_src_encoding,
                id.ctrl.vec_nf,
                id.ctrl.vec_broadcast_vs2,
            );
            // operand_groups doesn't have EEW/SEW for vec mem; override grp.vd / grp.vs2 here.
            let is_mem = is_vec_load(id.ctrl.vec_op) || is_vec_store(id.ctrl.vec_op);
            if is_mem {
                let vtype = parse_vtype(state.hart.csrs.vtype);
                if !vtype.vill {
                    grp.vd = vec_mem_dst_count(
                        id.ctrl.vec_op,
                        id.ctrl.vec_eew,
                        vtype.vsew,
                        vtype.vlmul,
                        id.ctrl.vec_nf,
                    );
                    let (_, idx_emul) =
                        vec_mem_emul_regs(id.ctrl.vec_op, id.ctrl.vec_eew, vtype.vsew, vtype.vlmul);
                    if idx_emul > 0 {
                        grp.vs2 = idx_emul;
                    } else {
                        grp.vs2 = 0;
                    }
                }
            }
            let mut vs1_phys = [VecPhysReg::ZERO; 8];
            let mut vs2_phys = [VecPhysReg::ZERO; 8];
            let mut vs3_phys = [VecPhysReg::ZERO; 8];
            let mut vec_src1_count: u8 = 0;
            let mut vec_src2_count: u8 = 0;
            let mut vec_src3_count: u8 = 0;

            if lmul > 0 {
                if grp.vs2 > 0 {
                    vec_src2_count = grp.vs2;
                    let vs2_base = id.ctrl.vs2.as_u8();
                    for (i, slot) in vs2_phys.iter_mut().enumerate().take(grp.vs2 as usize) {
                        *slot = engine.rename_map().get_vec(VRegIdx::new(vs2_base + i as u8));
                    }
                }

                if grp.vs1 > 0 {
                    vec_src1_count = grp.vs1;
                    let vs1_base = id.ctrl.vs1.as_u8();
                    for (i, slot) in vs1_phys.iter_mut().enumerate().take(grp.vs1 as usize) {
                        *slot = engine.rename_map().get_vec(VRegIdx::new(vs1_base + i as u8));
                    }
                }

                // vs3 = old vd; needed for tail/mask merging and as store data source.
                if grp.vd > 0 && (id.ctrl.vec_reg_write || is_vec_store(id.ctrl.vec_op)) {
                    vec_src3_count = grp.vd;
                    let vd_base = id.ctrl.vd.as_u8();
                    for (i, slot) in vs3_phys.iter_mut().enumerate().take(grp.vd as usize) {
                        *slot = engine.rename_map().get_vec(VRegIdx::new(vd_base + i as u8));
                    }
                }
            }

            // x0 stays unrenamed: hardwired zero, never freed at commit.
            let needs_dst = (id.ctrl.reg_write && !id.rd.is_zero()) || id.ctrl.fp_reg_write;
            let (rd_phys, old_phys_dst) = if needs_dst {
                let Some(new_p) = engine.free_list_mut().allocate() else {
                    input.push(id);
                    break;
                };
                let old_p = engine.rename_map().get(id.rd, id.ctrl.fp_reg_write);
                (new_p, old_p)
            } else {
                (PhysReg(0), PhysReg(0))
            };

            let vec_dst_count = if id.ctrl.vec_reg_write && grp.vd > 0 { grp.vd } else { 0 };
            if vec_dst_count > 0 && engine.vec_free_list_mut().available() < vec_dst_count as usize
            {
                if needs_dst {
                    engine.free_list_mut().reclaim(rd_phys);
                }
                budget = 0;
                input.push(id);
                continue;
            }

            let Some(rob_tag) = engine.rob_mut().allocate(
                id.pc,
                id.inst,
                id.inst_size,
                id.rd,
                id.ctrl.fp_reg_write,
                id.ctrl,
                rd_phys,
                old_phys_dst,
            ) else {
                if needs_dst {
                    engine.free_list_mut().reclaim(rd_phys);
                }
                input.push(id);
                break;
            };

            if needs_dst {
                engine.rename_map_mut().set(id.rd, id.ctrl.fp_reg_write, rd_phys);
                engine.prf_mut().allocate(rd_phys);
            }

            let mut vd_phys = [VecPhysReg::ZERO; 8];
            if vec_dst_count > 0 {
                let mut vec_old_phys = [VecPhysReg::ZERO; 8];
                let vd_base = id.ctrl.vd.as_u8();
                for i in 0..vec_dst_count as usize {
                    let vreg = VRegIdx::new(vd_base + i as u8);
                    let old_p = engine.rename_map().get_vec(vreg);
                    let Some(new_p) = engine.vec_free_list_mut().allocate() else {
                        unreachable!("vec free list pre-check guarantees capacity");
                    };
                    vec_old_phys[i] = old_p;
                    vd_phys[i] = new_p;
                    engine.rename_map_mut().set_vec(vreg, new_p);
                    engine.vec_prf_mut().allocate(new_p);
                }
                engine.rob_mut().set_vec_phys_dst(rob_tag, vd_phys, vec_old_phys, vec_dst_count);
            }

            if id.ctrl.mem_write {
                let width = id.ctrl.width;
                if !engine.store_buffer_mut().allocate(rob_tag, width) {
                    input.push(id);
                    break;
                }
            }

            if id.ctrl.mem_read
                && let Some(lq) = engine.load_queue_mut()
            {
                let width = id.ctrl.width;
                if !lq.allocate(rob_tag, width, None) {
                    input.push(id);
                    break;
                }
            }

            // Snapshot rename map *after* rd has been renamed.
            if is_branch_or_jump && engine.checkpoint_count() > 0 {
                let map_snapshot = engine.rename_map().clone();

                let Some(ckpt_id) = engine.checkpoint_table_mut().allocate(
                    rob_tag,
                    &map_snapshot,
                    state.hart.csrs.vtype,
                    state.hart.csrs.vl,
                    state.hart.csrs.frm,
                    state.hart.csrs.vxrm,
                    state.hart.csrs.vstart,
                ) else {
                    unreachable!("checkpoint table full after stall check");
                };

                engine.rob_mut().set_checkpoint_id(rob_tag, ckpt_id);
            }

            let entry = RenameIssueEntry {
                rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rs1: id.rs1,
                rs2: id.rs2,
                rs3: id.rs3,
                rd: id.rd,
                imm: id.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                rs1_phys,
                rs2_phys,
                rs3_phys,
                rd_phys,
                rs1_tag: None,
                rs2_tag: None,
                rs3_tag: None,
                ctrl: id.ctrl,
                trap: id.trap,
                exception_stage: id.exception_stage,
                pred_taken: id.pred_taken,
                pred_target: id.pred_target,
                ghr_snapshot: id.ghr_snapshot,
                ras_snapshot: id.ras_snapshot,
                vs1_phys,
                vs2_phys,
                vs3_phys,
                vd_phys,
                vec_src1_count,
                vec_src2_count,
                vec_src3_count,
                mask_phys: if !id.ctrl.vm && id.ctrl.vec_op != VectorOp::None {
                    engine.rename_map().get_vec(VRegIdx::new(0))
                } else {
                    VecPhysReg::ZERO
                },
                // Snapshot vector CSRs so execute uses dispatch-time context even after vsetvl.
                vec_vtype: state.hart.csrs.vtype,
                vec_vl: state.hart.csrs.vl,
                vec_vstart: state.hart.csrs.vstart,
                vec_vxrm: state.hart.csrs.vxrm,
                vec_frm: state.hart.csrs.frm,
            };

            trace_rename!(state.config.general.trace_instructions;
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.rob_tag.0,
                rd         = entry.rd.as_usize(),
                rd_phys    = rd_phys.0,
                old_phys   = old_phys_dst.0,
                rs1        = entry.rs1.as_usize(),
                rs1_phys   = rs1_phys.0,
                rs2        = entry.rs2.as_usize(),
                rs2_phys   = rs2_phys.0,
                is_store   = entry.ctrl.mem_write,
                is_load    = entry.ctrl.mem_read,
                is_fp      = entry.ctrl.fp_reg_write,
                "RN: O3 rename"
            );

            rename_output.push(entry);
        } else {
            let Some(rob_tag) = engine.rob_mut().allocate(
                id.pc,
                id.inst,
                id.inst_size,
                id.rd,
                id.ctrl.fp_reg_write,
                id.ctrl,
                PhysReg(0),
                PhysReg(0),
            ) else {
                input.push(id);
                break;
            };

            // Capture source tags BEFORE updating scoreboard for rd.
            let rs1_tag = engine.scoreboard().get_producer(id.rs1, id.ctrl.rs1_fp);
            let rs2_tag = engine.scoreboard().get_producer(id.rs2, id.ctrl.rs2_fp);
            let rs3_tag =
                if id.ctrl.rs3_fp { engine.scoreboard().get_producer(id.rs3, true) } else { None };

            if id.ctrl.reg_write || id.ctrl.fp_reg_write {
                engine.scoreboard_mut().set_producer(id.rd, id.ctrl.fp_reg_write, rob_tag);
            }

            if id.ctrl.mem_write {
                let width = id.ctrl.width;
                if !engine.store_buffer_mut().allocate(rob_tag, width) {
                    input.push(id);
                    break;
                }
            }

            let entry = RenameIssueEntry {
                rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rs1: id.rs1,
                rs2: id.rs2,
                rs3: id.rs3,
                rd: id.rd,
                imm: id.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                rs1_phys: PhysReg(0),
                rs2_phys: PhysReg(0),
                rs3_phys: PhysReg(0),
                rd_phys: PhysReg(0),
                rs1_tag,
                rs2_tag,
                rs3_tag,
                ctrl: id.ctrl,
                trap: id.trap,
                exception_stage: id.exception_stage,
                pred_taken: id.pred_taken,
                pred_target: id.pred_target,
                ghr_snapshot: id.ghr_snapshot,
                ras_snapshot: id.ras_snapshot,
                vs1_phys: [VecPhysReg::ZERO; 8],
                vs2_phys: [VecPhysReg::ZERO; 8],
                vs3_phys: [VecPhysReg::ZERO; 8],
                vd_phys: [VecPhysReg::ZERO; 8],
                vec_src1_count: 0,
                vec_src2_count: 0,
                vec_src3_count: 0,
                mask_phys: VecPhysReg::ZERO,
                vec_vtype: state.hart.csrs.vtype,
                vec_vl: state.hart.csrs.vl,
                vec_vstart: state.hart.csrs.vstart,
                vec_vxrm: state.hart.csrs.vxrm,
                vec_frm: state.hart.csrs.frm,
            };

            trace_rename!(state.config.general.trace_instructions;
                pc         = %crate::trace::Hex(entry.pc),
                rob_tag    = entry.rob_tag.0,
                rd         = entry.rd.as_usize(),
                rs1        = entry.rs1.as_usize(),
                rs1_tag    = ?entry.rs1_tag,
                rs2        = entry.rs2.as_usize(),
                rs2_tag    = ?entry.rs2_tag,
                is_store   = entry.ctrl.mem_write,
                is_load    = entry.ctrl.mem_read,
                "RN: in-order rename"
            );

            rename_output.push(entry);
        }
        budget -= 1;
    }
}
