//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR or VecPrfView).
//!
//! Two execution paths:
//! - **In-order / serializing:** `execute_vec_op()` — writes results to arch VPR
//!   and applies CSR side effects immediately (used by in-order backend and vsetvl).
//! - **O3 / deferred:** `execute_vec_op_on()` — writes results to `&mut impl VectorRegFile`
//!   (typically a `VecPrfView`) and returns side effects for commit-time application.

use crate::common::Trap;
use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::signals::{VecSrcEncoding, VectorOp};
use crate::core::units::vpu::alu::{VecExecCtx, VecOperand, vec_execute};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::core::units::vpu::types::{Vxrm, parse_vtype_with_elen};
use crate::core::units::vpu::vsetvl::execute_vsetvl;
use crate::core::units::vpu::{fpu, mask, mem, permute, reduction};
use crate::isa::rvv::encoding as v_enc;

/// Execute a vector operation. Returns the scalar result (for vsetvl family)
/// or 0 for arithmetic/memory ops.
///
/// # Errors
///
/// Returns `Trap::IllegalInstruction` if vtype.vill is set and a vector
/// operation that depends on vtype is attempted. Returns memory traps from
/// vector load/store operations.
pub fn execute_vec_op(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    match id.ctrl.vec_op {
        VectorOp::Vsetvli => Ok(execute_vsetvl_op(cpu, id)),
        VectorOp::Vsetivli => Ok(execute_vsetivli_op(cpu, id)),
        VectorOp::Vsetvl => Ok(execute_vsetvl_rs2_op(cpu, id)),
        VectorOp::None => Ok(0),
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
    cpu.csrs.vstart = 0;
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
    cpu.csrs.vstart = 0;
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
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    new_vl
}

/// Build the common execution context from CPU state.
fn build_ctx(cpu: &Cpu) -> VecExecCtx {
    let vtype = parse_vtype_with_elen(cpu.csrs.vtype, cpu.elen);
    VecExecCtx {
        sew: vtype.vsew,
        vl: cpu.csrs.vl as usize,
        vstart: cpu.csrs.vstart as usize,
        vma: vtype.vma,
        vta: vtype.vta,
        vlmul: vtype.vlmul,
        vm: true, // overridden per-instruction
        vxrm: Vxrm::from_bits(cpu.csrs.vxrm as u8),
        frm: match RoundingMode::from_bits(cpu.csrs.frm as u8) {
            Some(rm) => rm,
            None => RoundingMode::Rne,
        },
        zvfh: cpu.zvfh,
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
                VectorOp::VSll
                    | VectorOp::VSrl
                    | VectorOp::VSra
                    | VectorOp::VNSrl
                    | VectorOp::VNSra
                    | VectorOp::VNClipU
                    | VectorOp::VNClip
                    | VectorOp::VSSrl
                    | VectorOp::VSSra
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

/// Check vill and return `IllegalInstruction` trap if set.
///
/// RVV 1.0 §3.3: "If the vill bit is set, then any attempt to execute a
/// vector instruction that depends on vtype will raise an illegal-instruction
/// exception."
#[inline]
fn check_vill(inst: u32, vtype_bits: u64, elen: usize) -> Result<(), Trap> {
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    if vtype.vill {
        return Err(Trap::IllegalInstruction(inst));
    }
    Ok(())
}

/// Execute a vector integer arithmetic operation on the VPR.
fn execute_vec_arith(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, cpu.csrs.vtype, cpu.elen)?;

    let vtype = parse_vtype_with_elen(cpu.csrs.vtype, cpu.elen);
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
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector floating-point operation.
fn execute_vec_fp(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, cpu.csrs.vtype, cpu.elen)?;

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
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector reduction operation.
fn execute_vec_reduction(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, cpu.csrs.vtype, cpu.elen)?;

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
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector mask operation.
fn execute_vec_mask(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, cpu.csrs.vtype, cpu.elen)?;

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
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector permutation operation.
fn execute_vec_permute(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, cpu.csrs.vtype, cpu.elen)?;

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
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector load operation through the memory subsystem.
fn execute_vec_load(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let result = mem::execute_vec_load(cpu, id)?;
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    Ok(result)
}

/// Execute a vector store operation through the memory subsystem.
fn execute_vec_store(cpu: &mut Cpu, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let result = mem::execute_vec_store(cpu, id)?;
    cpu.csrs.vstart = 0;
    mark_vs_dirty(cpu);
    Ok(result)
}

// ──────────────────────────────────────────────────────────────────────
// O3 deferred execution path: operates on VecPrfView, returns side effects
// ──────────────────────────────────────────────────────────────────────

/// Side effects produced by a deferred vector execution.
///
/// These are NOT applied to CSRs immediately. The O3 backend stores them in
/// the ROB and applies them at commit time.
#[derive(Clone, Debug, Default)]
pub struct VecOpResult {
    /// Scalar result value (vsetvl -> new vl; vmv.x.s/vcpop.m/vfirst.m -> scalar).
    pub scalar_result: u64,
    /// FP exception flags (IEEE 754 NV/DZ/OF/UF/NX bits).
    pub fp_flags: u8,
    /// Fixed-point saturation flag (vxsat).
    pub vxsat: bool,
}

/// Build execution context from raw CSR values (no Cpu reference needed).
fn build_ctx_from_csrs(
    vtype_bits: u64,
    vl: u64,
    vstart: u64,
    vxrm: u64,
    frm: u64,
    elen: usize,
    zvfh: bool,
) -> VecExecCtx {
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    VecExecCtx {
        sew: vtype.vsew,
        vl: vl as usize,
        vstart: vstart as usize,
        vma: vtype.vma,
        vta: vtype.vta,
        vlmul: vtype.vlmul,
        vm: true, // overridden per-instruction
        vxrm: Vxrm::from_bits(vxrm as u8),
        frm: match RoundingMode::from_bits(frm as u8) {
            Some(rm) => rm,
            None => RoundingMode::Rne,
        },
        zvfh,
    }
}

/// Execute a non-memory, non-vsetvl vector operation on any `VectorRegFile`.
///
/// This is the O3 deferred execution path. It performs the functional
/// computation on the provided register file (typically a `VecPrfView`) and
/// returns the side effects (fp_flags, vxsat) without modifying any CSRs.
///
/// # Errors
///
/// Returns `Trap::IllegalInstruction` if vtype.vill is set.
///
/// # Panics
///
/// Panics if called with vsetvl or memory vector ops (those have separate paths).
pub fn execute_vec_op_on<V: VectorRegFile>(
    vpr: &mut V,
    vtype_bits: u64,
    vl: u64,
    vstart: u64,
    vxrm: u64,
    frm: u64,
    elen: usize,
    zvfh: bool,
    id: &RenameIssueEntry,
) -> Result<VecOpResult, Trap> {
    debug_assert!(
        !matches!(
            id.ctrl.vec_op,
            VectorOp::Vsetvli | VectorOp::Vsetivli | VectorOp::Vsetvl | VectorOp::None
        ),
        "execute_vec_op_on called with vsetvl/None — use execute_vec_op instead"
    );
    debug_assert!(
        !mem::is_vec_load(id.ctrl.vec_op) && !mem::is_vec_store(id.ctrl.vec_op),
        "execute_vec_op_on called with memory op — use generate_element_addrs_vrf instead"
    );

    check_vill(id.inst, vtype_bits, elen)?;

    let mut ctx = build_ctx_from_csrs(vtype_bits, vl, vstart, vxrm, frm, elen, zvfh);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);
    let vec_op = id.ctrl.vec_op;

    if fpu::is_vec_fp(vec_op) {
        let result = fpu::vec_fp_execute(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, operand1, &ctx);
        return Ok(VecOpResult {
            scalar_result: result.scalar_result.unwrap_or(0),
            fp_flags: result.fp_flags.bits() as u8,
            vxsat: false,
        });
    }

    if reduction::is_reduction(vec_op) {
        let operand1_ref = VecOperand::Vector(id.ctrl.vs1);
        let result = reduction::vec_reduce(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, &operand1_ref, &ctx);
        return Ok(VecOpResult {
            scalar_result: result.scalar_result.unwrap_or(0),
            fp_flags: result.fp_flags.bits() as u8,
            vxsat: false,
        });
    }

    if mask::is_mask_op(vec_op) {
        let result = mask::vec_mask_execute(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, &operand1, &ctx);
        return Ok(VecOpResult {
            scalar_result: result.scalar_result.unwrap_or(0),
            fp_flags: 0,
            vxsat: false,
        });
    }

    if permute::is_permute(vec_op) {
        let result = permute::vec_permute_execute(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, &operand1, &ctx);
        return Ok(VecOpResult {
            scalar_result: result.scalar_result.unwrap_or(0),
            fp_flags: 0,
            vxsat: false,
        });
    }

    // Integer arithmetic (default)
    let result = vec_execute(
        vec_op,
        vpr,
        id.ctrl.vd,
        id.ctrl.vs2,
        operand1,
        ctx.sew,
        ctx.vl,
        ctx.vstart,
        ctx.vma,
        ctx.vta,
        ctx.vlmul,
        id.ctrl.vm,
        ctx.vxrm,
    );

    Ok(VecOpResult {
        scalar_result: result.scalar_result.unwrap_or(0),
        fp_flags: 0,
        vxsat: result.vxsat,
    })
}
