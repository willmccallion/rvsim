//! Rename for the out-of-order engine: physical registers from the free
//! list, the speculative rename map, and a ROB, load-queue, store-buffer
//! and checkpoint slot per instruction.
//!
//! Source physical registers are captured before the destination is
//! renamed, so an instruction reading its own destination (`addi x5, x5,
//! 16`) sees the previous producer.

use super::O3Engine;
use crate::core::pipeline::engine::{ExecutionEngine, Renamed};
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::vec_prf::VecPhysReg;
use crate::exec::compute::vector::mem::{
    is_vec_load, is_vec_store, vec_mem_dst_count, vec_mem_emul_regs,
};
use crate::exec::inst::Inst;
use crate::exec::signals::ControlFlow;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{VRegIdx, parse_vtype};
use crate::sim::StageCtx;
use crate::trace_rename;

impl O3Engine {
    /// Renames one decoded instruction and allocates its backend slots.
    /// Returns the instruction when a slot it needs is short, before
    /// anything is allocated; `can_accept` covers only the ROB and issue
    /// queue slots every instruction needs, as gem5's rename checks each
    /// instruction's own resources.
    pub(super) fn rename_one(&mut self, state: &mut StageCtx<'_>, id: IdExEntry) -> Renamed {
        if !self.serialization.admits(self.cycle) {
            state.counter(state.core().stat_paths.pipeline.stalls_serialize).inc();
            return Renamed::Stalled(Box::new(id));
        }
        let vector = self.vector_config(&state.hart().csrs);
        let is_branch_or_jump =
            matches!(id.inst.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump);
        if is_branch_or_jump && self.checkpoints.capacity() > 0 && self.checkpoints.is_full() {
            state.counter(state.core().stat_paths.pipeline.stalls_checkpoint).inc();
            return Renamed::Stalled(Box::new(id));
        }
        if !self.has_slots_for(&id) {
            return Renamed::Stalled(Box::new(id));
        }

        // Capture source physical regs BEFORE updating rename map for rd.
        let rs1_phys = self.rename_map.get(id.inst.rs1, id.inst.ctrl.rs1_fp);
        let rs2_phys = self.rename_map.get(id.inst.rs2, id.inst.ctrl.rs2_fp);
        let rs3_phys =
            if id.inst.ctrl.rs3_fp { self.rename_map.get(id.inst.rs3, true) } else { PhysReg(0) };

        // Capture vector source mappings before vd is renamed.
        let lmul = id.inst.ctrl.vec_lmul_regs;
        let mut grp = id.inst.ctrl.vec_op.operand_groups(
            lmul,
            id.inst.ctrl.vec_lmul_is_fractional,
            id.inst.ctrl.vec_src_encoding,
            id.inst.ctrl.vec_nf,
            id.inst.ctrl.vec_broadcast_vs2,
        );
        // operand_groups doesn't have EEW/SEW for vec mem; override grp.vd / grp.vs2 here.
        let is_mem = is_vec_load(id.inst.ctrl.vec_op) || is_vec_store(id.inst.ctrl.vec_op);
        if is_mem {
            let vtype = parse_vtype(vector.vtype);
            if !vtype.vill {
                grp.vd = vec_mem_dst_count(
                    id.inst.ctrl.vec_op,
                    id.inst.ctrl.vec_eew,
                    vtype.vsew,
                    vtype.vlmul,
                    id.inst.ctrl.vec_nf,
                );
                let (_, idx_emul) = vec_mem_emul_regs(
                    id.inst.ctrl.vec_op,
                    id.inst.ctrl.vec_eew,
                    vtype.vsew,
                    vtype.vlmul,
                );
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
                let vs2_base = id.inst.ctrl.vs2.as_u8();
                for (i, slot) in vs2_phys.iter_mut().enumerate().take(grp.vs2 as usize) {
                    *slot = self.rename_map.get_vec(VRegIdx::new(vs2_base + i as u8));
                }
            }

            if grp.vs1 > 0 {
                vec_src1_count = grp.vs1;
                let vs1_base = id.inst.ctrl.vs1.as_u8();
                for (i, slot) in vs1_phys.iter_mut().enumerate().take(grp.vs1 as usize) {
                    *slot = self.rename_map.get_vec(VRegIdx::new(vs1_base + i as u8));
                }
            }

            // vs3 = old vd; needed for tail/mask merging and as store data source.
            if grp.vd > 0 && (id.inst.ctrl.vec_reg_write || is_vec_store(id.inst.ctrl.vec_op)) {
                vec_src3_count = grp.vd;
                let vd_base = id.inst.ctrl.vd.as_u8();
                for (i, slot) in vs3_phys.iter_mut().enumerate().take(grp.vd as usize) {
                    *slot = self.rename_map.get_vec(VRegIdx::new(vd_base + i as u8));
                }
            }
        }

        // x0 stays unrenamed: hardwired zero, never freed at commit.
        let needs_dst =
            (id.inst.ctrl.reg_write && !id.inst.rd.is_zero()) || id.inst.ctrl.fp_reg_write;
        let (rd_phys, old_phys_dst) = if needs_dst {
            let Some(new_p) = self.free_list.allocate() else {
                return Renamed::Stalled(Box::new(id));
            };
            let old_p = self.rename_map.get(id.inst.rd, id.inst.ctrl.fp_reg_write);
            (new_p, old_p)
        } else {
            (PhysReg(0), PhysReg(0))
        };

        let vec_dst_count = if id.inst.ctrl.vec_reg_write && grp.vd > 0 { grp.vd } else { 0 };
        if vec_dst_count > 0 && self.vec_free_list.available() < vec_dst_count as usize {
            if needs_dst {
                self.free_list.reclaim(rd_phys);
            }
            return Renamed::Stalled(Box::new(id));
        }

        let Some(rob_tag) = self.rob.allocate(
            id.inst.pc,
            id.inst.bits,
            id.inst.size,
            id.inst.rd,
            id.inst.ctrl.fp_reg_write,
            id.inst.ctrl,
            rd_phys,
            old_phys_dst,
            id.seq,
        ) else {
            if needs_dst {
                self.free_list.reclaim(rd_phys);
            }
            return Renamed::Stalled(Box::new(id));
        };

        if needs_dst {
            self.rename_map.set(id.inst.rd, id.inst.ctrl.fp_reg_write, rd_phys);
            self.prf.allocate(rd_phys);
        }

        let mut vd_phys = [VecPhysReg::ZERO; 8];
        if vec_dst_count > 0 {
            let mut vec_old_phys = [VecPhysReg::ZERO; 8];
            let vd_base = id.inst.ctrl.vd.as_u8();
            for i in 0..vec_dst_count as usize {
                let vreg = VRegIdx::new(vd_base + i as u8);
                let old_p = self.rename_map.get_vec(vreg);
                let Some(new_p) = self.vec_free_list.allocate() else {
                    unreachable!("vec free list pre-check guarantees capacity");
                };
                vec_old_phys[i] = old_p;
                vd_phys[i] = new_p;
                self.rename_map.set_vec(vreg, new_p);
                self.vec_prf.allocate(new_p);
            }
            self.rob.set_vec_phys_dst(rob_tag, vd_phys, vec_old_phys, vec_dst_count);
        }

        let store_slot_allocated = if id.inst.ctrl.uses_store_buffer() {
            self.store_buffer.allocate(rob_tag, id.inst.ctrl.width)
        } else if is_vec_store(id.inst.ctrl.vec_op) {
            self.vec_store_buffer.allocate(rob_tag)
        } else {
            true
        };
        let load_slot_allocated = !id.inst.ctrl.mem_read
            || self.load_queue.allocate(rob_tag, id.inst.ctrl.width.bytes() as usize, None);
        debug_assert!(
            store_slot_allocated && load_slot_allocated,
            "has_slots_for checked the memory slots"
        );

        // Snapshot rename map *after* rd has been renamed.
        if is_branch_or_jump && self.checkpoints.capacity() > 0 {
            let map_snapshot = self.rename_map.clone();

            let Some(ckpt_id) = self.checkpoints.allocate(rob_tag, &map_snapshot) else {
                unreachable!("checkpoint table full after stall check");
            };

            self.rob.set_checkpoint_id(rob_tag, ckpt_id);
        }

        let entry = RenameIssueEntry {
            inst: Inst {
                pc: id.inst.pc,
                bits: id.inst.bits,
                size: id.inst.size,
                rs1: id.inst.rs1,
                rs2: id.inst.rs2,
                rs3: id.inst.rs3,
                rd: id.inst.rd,
                imm: id.inst.imm,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl: id.inst.ctrl,
            },
            rob_tag,
            rs1_phys,
            rs2_phys,
            rs3_phys,
            rd_phys,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            trap: id.trap,
            exception_stage: id.exception_stage,
            pred_taken: id.pred_taken,
            pred_target: id.pred_target,
            seq: id.seq,
            vs1_phys,
            vs2_phys,
            vs3_phys,
            vd_phys,
            vec_src1_count,
            vec_src2_count,
            vec_src3_count,
            mask_phys: if !id.inst.ctrl.vm && id.inst.ctrl.vec_op != VectorOp::None {
                self.rename_map.get_vec(VRegIdx::new(0))
            } else {
                VecPhysReg::ZERO
            },
            // Snapshot vector CSRs so execute uses dispatch-time context even after vsetvl.
            vec_vtype: vector.vtype,
            vec_vl: vector.vl,
            vec_vstart: vector.vstart,
            vec_vxrm: state.hart().csrs.vxrm,
            vec_frm: state.hart().csrs.frm,
        };

        trace_rename!(state.config.general.trace_instructions;
            pc         = %crate::trace::Hex(entry.inst.pc),
            rob_tag    = entry.rob_tag.0,
            rd         = entry.inst.rd.as_usize(),
            rd_phys    = rd_phys.0,
            old_phys   = old_phys_dst.0,
            rs1        = entry.inst.rs1.as_usize(),
            rs1_phys   = rs1_phys.0,
            rs2        = entry.inst.rs2.as_usize(),
            rs2_phys   = rs2_phys.0,
            is_store   = entry.inst.ctrl.mem_write,
            is_load    = entry.inst.ctrl.mem_read,
            is_fp      = entry.inst.ctrl.fp_reg_write,
            "RN: O3 rename"
        );

        if entry.inst.ctrl.system_op.serializes_after() {
            self.serialization = super::serialize::Serialization::after(entry.rob_tag);
        }
        Renamed::Accepted(Box::new(entry))
    }

    /// True when the backend has the slots `id` needs besides the ROB and
    /// issue-queue slots `can_accept` covers: a physical register for a
    /// destination, and a store-buffer, vector-store-buffer or load-queue
    /// slot for a memory op.
    fn has_slots_for(&self, id: &IdExEntry) -> bool {
        let needs_dst =
            (id.inst.ctrl.reg_write && !id.inst.rd.is_zero()) || id.inst.ctrl.fp_reg_write;
        let store_slot = if id.inst.ctrl.uses_store_buffer() {
            !self.store_buffer.is_full()
        } else if is_vec_store(id.inst.ctrl.vec_op) {
            self.vec_store_buffer.free_slots() > 0
        } else {
            true
        };
        let load_slot = !id.inst.ctrl.mem_read || !self.load_queue.is_full();
        let register = !needs_dst || self.free_list.available() > 0;
        store_slot && load_slot && register
    }
}
