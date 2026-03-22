//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU ALU
//! (which operates on the architectural VPR). All vector ops are serializing,
//! so we can safely read/write the VPR at execute time.

use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::signals::{VecSrcEncoding, VectorOp};
use crate::core::units::vpu::alu::{VecOperand, vec_execute};
use crate::core::units::vpu::mem;
use crate::core::units::vpu::types::{Vlmax, parse_vtype};
use crate::core::units::vpu::vsetvl::execute_vsetvl;
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
    new_vl
}

/// Execute a vector arithmetic operation on the VPR.
fn execute_vec_arith(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    let vtype = parse_vtype(cpu.csrs.vtype);

    // If vtype is illegal, the instruction is a no-op (spec: vill=1 means
    // all vector arithmetic instructions raise illegal instruction).
    // However, since we already decoded it, we treat it as a NOP here.
    // In a real implementation, this would trap. For now, just return 0.
    if vtype.vill {
        return 0;
    }

    let vl = cpu.csrs.vl as usize;
    let vstart = cpu.csrs.vstart as usize;
    let vlen = cpu.regs.vpr().vlen();
    let _vlmax = Vlmax::compute(vlen, vtype.vsew, vtype.vlmul).as_usize();

    // Build operand1 based on source encoding
    let operand1 = match id.ctrl.vec_src_encoding {
        VecSrcEncoding::VV => VecOperand::Vector(id.ctrl.vs1),
        VecSrcEncoding::VX => VecOperand::Scalar(id.rv1),
        VecSrcEncoding::VI => VecOperand::Immediate(v_enc::simm5(id.inst)),
        VecSrcEncoding::VF | VecSrcEncoding::None => VecOperand::Scalar(0),
    };

    let result = vec_execute(
        id.ctrl.vec_op,
        cpu.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        operand1,
        vtype.vsew,
        vl,
        vstart,
        vtype.vma,
        vtype.vta,
        vtype.vlmul,
        id.ctrl.vm,
        crate::core::units::vpu::types::Vxrm::from_bits(cpu.csrs.vxrm as u8),
    );

    // Apply vxsat if saturation occurred
    if result.vxsat {
        cpu.csrs.vxsat = 1;
    }

    // Clear vstart after successful execution
    cpu.csrs.vstart = 0;

    // Set mstatus.VS = Dirty
    cpu.csrs.mstatus = (cpu.csrs.mstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;
    cpu.csrs.sstatus = (cpu.csrs.sstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;

    // Vector arithmetic ops produce no scalar result
    result.scalar_result.unwrap_or(0)
}

/// Execute a vector load operation through the memory subsystem.
fn execute_vec_load(cpu: &mut Cpu, id: &RenameIssueEntry) -> u64 {
    match mem::execute_vec_load(cpu, id) {
        Ok(result) => {
            // Clear vstart after successful execution
            cpu.csrs.vstart = 0;
            // Set mstatus.VS = Dirty
            cpu.csrs.mstatus = (cpu.csrs.mstatus & !crate::core::arch::csr::MSTATUS_VS)
                | crate::core::arch::csr::MSTATUS_VS_DIRTY;
            cpu.csrs.sstatus = (cpu.csrs.sstatus & !crate::core::arch::csr::MSTATUS_VS)
                | crate::core::arch::csr::MSTATUS_VS_DIRTY;
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
            result
        }
        Err(_trap) => {
            // TODO: propagate trap through pipeline
            0
        }
    }
}
