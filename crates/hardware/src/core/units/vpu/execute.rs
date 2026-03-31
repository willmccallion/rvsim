//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR). All vector ops
//! are serializing, so we can safely read/write the VPR at execute time.

use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::signals::{VecSrcEncoding, VectorOp};
use crate::core::units::vpu::alu::{VecExecCtx, VecOperand, vec_execute};
use crate::core::units::vpu::types::{Vxrm, parse_vtype};
use crate::core::units::vpu::vsetvl::execute_vsetvl;
use crate::core::units::vpu::{fpu, mask, mem, permute, reduction};
use crate::isa::rvv::encoding as v_enc;

/// Execute a vector operation. Returns the scalar result (for vsetvl family)
/// or 0 for arithmetic/memory ops.
pub fn execute_vec_op(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    match id.ctrl.vec_op {
        VectorOp::Vsetvli => execute_vsetvl_op(cpu, id),
        VectorOp::Vsetivli => execute_vsetivli_op(cpu, id),
        VectorOp::Vsetvl => execute_vsetvl_rs2_op(cpu, id),
        VectorOp::None => 0,
        op if mem::is_vec_load(op) => execute_vec_load(cpu, id),
        op if mem::is_vec_store(op) => execute_vec_store(cpu, id),
        op if fpu::is_vec_fp(op) => execute_vec_fp(cpu, id),
        op if reduction::is_reduction(op) => execute_vec_reduction(cpu, id),
        op if mask::is_mask_op(op) => execute_vec_mask(cpu, id),
        op if permute::is_permute(op) => execute_vec_permute(cpu, id),
        _ => execute_vec_arith(cpu, id),
    }
}

/// Execute `vsetvli`: AVL from rs1, vtype from immediate.
fn execute_vsetvl_op(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let avl = id.rv1;
    let requested_vtype = v_enc::zimm_vsetvli(id.inst);
    let rd_is_zero = id.rd.is_zero();
    let rs1_is_zero = id.rs1.is_zero();
    let vlen = cpu.regs.vpr().vlen();
    let current_vl = cpu.csrs.vl;

    let (new_vl, new_vtype) =
        execute_vsetvl(avl, requested_vtype, rd_is_zero, rs1_is_zero, vlen, current_vl);
    cpu.csrs.vl = new_vl;
    cpu.csrs.vtype = new_vtype;
    mark_vs_dirty(cpu);
    new_vl
}

/// Execute `vsetivli`: AVL from uimm, vtype from immediate.
fn execute_vsetivli_op(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let avl = v_enc::uimm_vsetivli(id.inst);
    let requested_vtype = v_enc::zimm_vsetivli(id.inst);
    let rd_is_zero = id.rd.is_zero();
    let vlen = cpu.regs.vpr().vlen();
    let current_vl = cpu.csrs.vl;

    // vsetivli: rs1_is_zero is always false (uimm provides AVL)
    let (new_vl, new_vtype) =
        execute_vsetvl(avl, requested_vtype, rd_is_zero, false, vlen, current_vl);
    cpu.csrs.vl = new_vl;
    cpu.csrs.vtype = new_vtype;
    mark_vs_dirty(cpu);
    new_vl
}

/// Execute `vsetvl`: AVL from rs1, vtype from rs2.
fn execute_vsetvl_rs2_op(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let avl = id.rv1;
    let requested_vtype = id.rv2;
    let rd_is_zero = id.rd.is_zero();
    let rs1_is_zero = id.rs1.is_zero();
    let vlen = cpu.regs.vpr().vlen();
    let current_vl = cpu.csrs.vl;

    let (new_vl, new_vtype) =
        execute_vsetvl(avl, requested_vtype, rd_is_zero, rs1_is_zero, vlen, current_vl);
    cpu.csrs.vl = new_vl;
    cpu.csrs.vtype = new_vtype;
    mark_vs_dirty(cpu);
    new_vl
}

/// Build the common execution context from CPU state.
const fn build_ctx(cpu: &Cpu) -> VecExecCtx {
    let vtype = parse_vtype(cpu.csrs.vtype);
    VecExecCtx {
        sew: vtype.vsew,
        vl: cpu.csrs.vl as usize,
        vstart: cpu.csrs.vstart as usize,
        vma: vtype.vma,
        vta: vtype.vta,
        vlmul: vtype.vlmul,
        vm: true, // overridden per-instruction
        vxrm: Vxrm::from_bits(cpu.csrs.vxrm as u8),
    }
}

/// Build operand1 from pipeline latch data based on source encoding.
///
/// Shift ops (vsll, vsrl, vsra, vnsrl, vnsra, vnclip, vnclip, vssrl, vssra)
/// use an unsigned 5-bit immediate per RVV 1.0 §11.7, while other OPIVI
/// instructions use a sign-extended immediate.
const fn build_operand1(id: &RenameIssueEntry) -> VecOperand {
    match id.ctrl.vec_src_encoding {
        VecSrcEncoding::VV => VecOperand::Vector(id.ctrl.vs1),
        VecSrcEncoding::VX | VecSrcEncoding::VF => VecOperand::Scalar(id.rv1),
        VecSrcEncoding::VI => {
            let uses_uimm = matches!(
                id.ctrl.vec_op,
                VectorOp::VSll | VectorOp::VSrl | VectorOp::VSra |
                VectorOp::VNSrl | VectorOp::VNSra |
                VectorOp::VNClipU | VectorOp::VNClip |
                VectorOp::VSSrl | VectorOp::VSSra
            );
            if uses_uimm {
                VecOperand::Immediate(v_enc::uimm5(id.inst) as i64)
            } else {
                VecOperand::Immediate(v_enc::simm5(id.inst))
            }
        }
        VecSrcEncoding::None => VecOperand::Scalar(0),
    }
}

/// Mark `mstatus.VS` and `sstatus.VS` as dirty.
const fn mark_vs_dirty(cpu: &mut Cpu) {
    cpu.csrs.mstatus = (cpu.csrs.mstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;
    cpu.csrs.sstatus = (cpu.csrs.sstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;
}

/// Execute a vector integer arithmetic operation on the VPR.
fn execute_vec_arith(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return 0;
    }

    let mut ctx = build_ctx(cpu);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = vec_execute(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        operand1,
        vtype.vsew,
        ctx.vl,
        ctx.vstart,
        vtype.vma,
        vtype.vta,
        vtype.vlmul,
        id.ctrl.vm,
        ctx.vxrm,
    );

    if result.vxsat {
        cpu.csrs.vxsat = 1;
    }
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector floating-point operation.
fn execute_vec_fp(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return 0;
    }

    let mut ctx = build_ctx(cpu);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = fpu::vec_fp_execute(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        operand1,
        &ctx,
    );

    // Accumulate FP exception flags
    cpu.csrs.fflags |= result.fp_flags.bits() as u64;
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector reduction operation.
fn execute_vec_reduction(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return 0;
    }

    let mut ctx = build_ctx(cpu);
    ctx.vm = id.ctrl.vm;

    let operand1 = VecOperand::Vector(id.ctrl.vs1);
    let result = reduction::vec_reduce(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    cpu.csrs.fflags |= result.fp_flags.bits() as u64;
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector mask operation.
fn execute_vec_mask(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return 0;
    }

    let mut ctx = build_ctx(cpu);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = mask::vec_mask_execute(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector permutation operation.
fn execute_vec_permute(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);
    if vtype.vill {
        return 0;
    }

    let mut ctx = build_ctx(cpu);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = permute::vec_permute_execute(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector load operation through the memory subsystem.
fn execute_vec_load(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    match mem::execute_vec_load(cpu, id) {
        Ok(result) => {
            cpu.csrs.vstart = 0;
            mark_vs_dirty(cpu);
            result
        }
        Err(_trap) => {
            // TODO: propagate trap through pipeline
            0
        }
    }
}

/// Execute a vector store operation through the memory subsystem.
fn execute_vec_store(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    match mem::execute_vec_store(cpu, id) {
        Ok(result) => {
            cpu.csrs.vstart = 0;
            mark_vs_dirty(cpu);
            result
        }
        Err(_trap) => {
            // TODO: propagate trap through pipeline
            0
        }
    }
}
