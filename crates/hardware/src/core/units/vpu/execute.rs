//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR or `VecPrfView`).
//!
//! Two execution paths:
//! - **In-order / serializing:** `execute_vec_op()` — writes results to arch VPR
//!   and applies CSR side effects immediately (used by in-order backend and vsetvl).
//! - **O3 / deferred:** `execute_vec_op_on()` — writes results to `&mut impl VectorRegFile`
//!   (typically a `VecPrfView`) and returns side effects for commit-time application.

use crate::common::Trap;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::signals::{VecSrcEncoding, VectorOp};
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::core::units::vpu::alu::{VecExecCtx, VecOperand, vec_execute};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{Vlmul, Vxrm, parse_vtype_with_elen};
use crate::core::units::vpu::{crypto, fpu, mask, mem, permute, reduction};
use crate::isa::rvv::encoding as v_enc;
use crate::sim::CoreCtx;

/// Execute a vector operation against the architectural vector registers.
/// `vsetvl` is not executed here: the backends record its result on the ROB.
///
/// # Errors
///
/// Returns `Trap::IllegalInstruction` if vtype.vill is set and a vector
/// operation that depends on vtype is attempted. Returns memory traps from
/// vector load/store operations.
pub fn execute_vec_op(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    match id.ctrl.vec_op {
        VectorOp::Vsetvli | VectorOp::Vsetivli | VectorOp::Vsetvl | VectorOp::None => Ok(0),
        op if mem::is_vec_load(op) => execute_vec_load(state, id),
        op if mem::is_vec_store(op) => execute_vec_store(state, id),
        op if fpu::is_vec_fp(op) => execute_vec_fp(state, id),
        op if reduction::is_reduction(op) => execute_vec_reduction(state, id),
        op if mask::is_mask_op(op) => execute_vec_mask(state, id),
        op if permute::is_permute(op) => execute_vec_permute(state, id),
        op if crypto::is_crypto(op) => execute_vec_crypto(state, id),
        _ => execute_vec_arith(state, id),
    }
}

/// Execute a vector crypto instruction (Zvkn*/Zvks*/Zvkg).
fn execute_vec_crypto(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;
    let vstart = state.hart.csrs.vstart as usize;
    let vl = state.hart.csrs.vl as usize;
    crypto::execute_crypto(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        id.ctrl.vs1,
        vstart,
        vl,
        id.inst,
        id.ctrl.vec_broadcast_vs2,
    );
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(0)
}

/// Build the common execution context from CPU state.
fn build_ctx(state: &CoreCtx<'_>) -> VecExecCtx {
    let vtype = parse_vtype_with_elen(state.hart.csrs.vtype, state.config.isa.vector.elen);
    VecExecCtx {
        sew: vtype.vsew,
        vl: state.hart.csrs.vl as usize,
        vstart: state.hart.csrs.vstart as usize,
        vma: vtype.vma,
        vta: vtype.vta,
        vlmul: vtype.vlmul,
        vm: true, // overridden per-instruction
        vxrm: Vxrm::from_bits(state.hart.csrs.vxrm as u8),
        frm: RoundingMode::from_bits(state.hart.csrs.frm as u8).unwrap_or(RoundingMode::Rne),
        zvfh: state.config.isa.vector.zvfh,
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
                    // Slides and gather take a zero-extended 5-bit
                    // immediate, not a sign-extended one.
                    | VectorOp::VSlideUp
                    | VectorOp::VSlideDown
                    | VectorOp::VRgather
                    // Zvbb shifts/rotates use unsigned immediates.
                    | VectorOp::VWsll
                    | VectorOp::VRor
            );
            if uses_uimm {
                // vror.vi splits a 6-bit imm as bit26 (zimm6hi) + bits[19:15] (zimm6lo).
                let imm = if matches!(id.ctrl.vec_op, VectorOp::VRor) {
                    let lo = v_enc::uimm5(id.inst);
                    let hi = ((id.inst >> 26) & 1) as u64;
                    ((hi << 5) | lo) as i64
                } else {
                    v_enc::uimm5(id.inst) as i64
                };
                VecOperand::Immediate(imm)
            } else {
                VecOperand::Immediate(v_enc::simm5(id.inst))
            }
        }
        VecSrcEncoding::None => VecOperand::Scalar(0),
    }
}

/// Mark `mstatus.VS` and `sstatus.VS` as dirty.
const fn mark_vs_dirty(state: &mut CoreCtx<'_>) {
    state.hart.csrs.mstatus = (state.hart.csrs.mstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;
    state.hart.csrs.sstatus = (state.hart.csrs.sstatus & !crate::core::arch::csr::MSTATUS_VS)
        | crate::core::arch::csr::MSTATUS_VS_DIRTY;
}

/// Check vill and return `IllegalInstruction` trap if set.
///
/// RVV 1.0 §3.3: "If the vill bit is set, then any attempt to execute a
/// vector instruction that depends on vtype will raise an illegal-instruction
/// exception."
#[inline]
const fn check_vill(inst: u32, vtype_bits: u64, elen: usize) -> Result<(), Trap> {
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    if vtype.vill {
        return Err(Trap::IllegalInstruction(inst));
    }
    Ok(())
}

/// Returns `true` if `op` widens (or reads/writes a 2×SEW operand) and would
/// therefore require `EMUL = 2 × LMUL`.
const fn op_uses_widened_emul(op: VectorOp) -> bool {
    matches!(
        op,
        // Integer/Zvbb widening
        VectorOp::VWAddU | VectorOp::VWAdd | VectorOp::VWSubU | VectorOp::VWSub
        | VectorOp::VWAddUW | VectorOp::VWAddW | VectorOp::VWSubUW | VectorOp::VWSubW
        | VectorOp::VWMulU | VectorOp::VWMul | VectorOp::VWMulSU
        | VectorOp::VWMaccU | VectorOp::VWMacc | VectorOp::VWMaccSU | VectorOp::VWMaccUS
        | VectorOp::VWsll
        // FP widening arithmetic + FMA
        | VectorOp::VFWAdd | VectorOp::VFWSub | VectorOp::VFWMul
        | VectorOp::VFWAddW | VectorOp::VFWSubW
        | VectorOp::VFWMacc | VectorOp::VFWNMacc | VectorOp::VFWMSac | VectorOp::VFWNMSac
        // FP widening conversions
        | VectorOp::VFWCvtXuF | VectorOp::VFWCvtXF
        | VectorOp::VFWCvtFXu | VectorOp::VFWCvtFX | VectorOp::VFWCvtFF
        | VectorOp::VFWCvtRtzXuF | VectorOp::VFWCvtRtzXF
        // Narrowing reads vs2 at 2×SEW
        | VectorOp::VNSrl | VectorOp::VNSra | VectorOp::VNClipU | VectorOp::VNClip
        | VectorOp::VFNCvtXuF | VectorOp::VFNCvtXF
        | VectorOp::VFNCvtFXu | VectorOp::VFNCvtFX | VectorOp::VFNCvtFF
        | VectorOp::VFNCvtRodFF
        | VectorOp::VFNCvtRtzXuF | VectorOp::VFNCvtRtzXF
    )
}

/// Reject widening / narrowing instructions when `2 × LMUL` would exceed the
/// architectural ceiling of 8 register-group registers.
///
/// Widening reductions are excluded: their destination is a single-register
/// scalar accumulator, so the EMUL ≤ 8 ceiling does not apply.
#[inline]
const fn check_widening_lmul(inst: u32, op: VectorOp, vlmul: Vlmul) -> Result<(), Trap> {
    if matches!(vlmul, Vlmul::M8) && op_uses_widened_emul(op) {
        return Err(Trap::IllegalInstruction(inst));
    }
    Ok(())
}

/// Execute a vector integer arithmetic operation on the VPR.
fn execute_vec_arith(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;
    let vtype = parse_vtype_with_elen(state.hart.csrs.vtype, state.config.isa.vector.elen);
    check_widening_lmul(id.inst, id.ctrl.vec_op, vtype.vlmul)?;
    let mut ctx = build_ctx(state);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = vec_execute(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
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
        state.hart.csrs.vxsat = 1;
    }
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector floating-point operation.
fn execute_vec_fp(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;
    let vtype = parse_vtype_with_elen(state.hart.csrs.vtype, state.config.isa.vector.elen);
    check_widening_lmul(id.inst, id.ctrl.vec_op, vtype.vlmul)?;

    let mut ctx = build_ctx(state);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = fpu::vec_fp_execute(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        operand1,
        &ctx,
    );

    state.hart.csrs.fflags |= result.fp_flags.bits() as u64;
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector reduction operation.
fn execute_vec_reduction(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;

    let mut ctx = build_ctx(state);
    ctx.vm = id.ctrl.vm;

    let operand1 = VecOperand::Vector(id.ctrl.vs1);
    let result = reduction::vec_reduce(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    state.hart.csrs.fflags |= result.fp_flags.bits() as u64;
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector mask operation.
fn execute_vec_mask(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;

    let mut ctx = build_ctx(state);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = mask::vec_mask_execute(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector permutation operation.
fn execute_vec_permute(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    check_vill(id.inst, state.hart.csrs.vtype, state.config.isa.vector.elen)?;

    let mut ctx = build_ctx(state);
    ctx.vm = id.ctrl.vm;
    let operand1 = build_operand1(id);

    let result = permute::vec_permute_execute(
        id.ctrl.vec_op,
        state.hart.regs.vpr_mut(),
        id.ctrl.vd,
        id.ctrl.vs2,
        &operand1,
        &ctx,
    );

    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result.scalar_result.unwrap_or(0))
}

/// Execute a vector load operation through the memory subsystem.
fn execute_vec_load(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let result = mem::execute_vec_load(state, id)?;
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result)
}

/// Execute a vector store operation through the memory subsystem.
fn execute_vec_store(state: &mut CoreCtx<'_>, id: &RenameIssueEntry) -> Result<u64, Trap> {
    let result = mem::execute_vec_store(state, id)?;
    state.hart.csrs.vstart = 0;
    mark_vs_dirty(state);
    Ok(result)
}

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

/// Build execution context from raw CSR values (no `SimState` reference needed).
const fn build_ctx_from_csrs(
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
/// returns the side effects (`fp_flags`, vxsat) without modifying any CSRs.
///
/// # Errors
///
/// Returns `Trap::IllegalInstruction` if vtype.vill is set.
///
/// # Panics
///
/// Panics if called with vsetvl or memory vector ops (those have separate paths).
#[allow(clippy::too_many_arguments)]
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
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    check_widening_lmul(id.inst, id.ctrl.vec_op, vtype.vlmul)?;

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
        let result =
            reduction::vec_reduce(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, &operand1_ref, &ctx);
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
        let result =
            permute::vec_permute_execute(vec_op, vpr, id.ctrl.vd, id.ctrl.vs2, &operand1, &ctx);
        return Ok(VecOpResult {
            scalar_result: result.scalar_result.unwrap_or(0),
            fp_flags: 0,
            vxsat: false,
        });
    }

    if crypto::is_crypto(vec_op) {
        crypto::execute_crypto(
            vec_op,
            vpr,
            id.ctrl.vd,
            id.ctrl.vs2,
            id.ctrl.vs1,
            ctx.vstart,
            ctx.vl,
            id.inst,
            id.ctrl.vec_broadcast_vs2,
        );
        return Ok(VecOpResult { scalar_result: 0, fp_flags: 0, vxsat: false });
    }

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
