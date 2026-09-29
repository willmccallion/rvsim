//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR or `VecPrfView`).
//!
//! `execute_vec_op_on()` writes results to a `&mut impl VectorRegFile` (a
//! `VecPrfView` on the O3 backend, a `ShadowVpr` on the in-order one) and
//! returns the side effects for commit-time application.

use crate::exec::compute::vector::alu::{VecExecCtx, VecExecResult, VecOperand, vec_execute};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::exec::compute::vector::{crypto, fpu, mask, mem, permute, reduction};
use crate::exec::inst::Inst;
use crate::isa::encoding::rvv::encoding as v_enc;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::{VecSrcEncoding, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{Vlmul, Vxrm, parse_vtype_with_elen};

/// Build operand1 from pipeline latch data based on source encoding.
///
/// Shift ops (vsll, vsrl, vsra, vnsrl, vnsra, vnclip, vnclip, vssrl, vssra)
/// use an unsigned 5-bit immediate per RVV 1.0 §11.7, while other OPIVI
/// instructions use a sign-extended immediate.
const fn build_operand1(inst: &Inst) -> VecOperand {
    match inst.ctrl.vec_src_encoding {
        VecSrcEncoding::VV => VecOperand::Vector(inst.ctrl.vs1),
        VecSrcEncoding::VX | VecSrcEncoding::VF => VecOperand::Scalar(inst.rv1),
        VecSrcEncoding::VI => {
            let uses_uimm = matches!(
                inst.ctrl.vec_op,
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
                let imm = if matches!(inst.ctrl.vec_op, VectorOp::VRor) {
                    let lo = v_enc::uimm5(inst.bits);
                    let hi = ((inst.bits >> 26) & 1) as u64;
                    ((hi << 5) | lo) as i64
                } else {
                    v_enc::uimm5(inst.bits) as i64
                };
                VecOperand::Immediate(imm)
            } else {
                VecOperand::Immediate(v_enc::simm5(inst.bits))
            }
        }
        VecSrcEncoding::None => VecOperand::Scalar(0),
    }
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

/// Build execution context from raw CSR values (no `SystemState` reference needed).
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
    inst: &Inst,
) -> Result<VecExecResult, Trap> {
    debug_assert!(
        !matches!(
            inst.ctrl.vec_op,
            VectorOp::Vsetvli | VectorOp::Vsetivli | VectorOp::Vsetvl | VectorOp::None
        ),
        "execute_vec_op_on called with vsetvl/None"
    );
    debug_assert!(
        !mem::is_vec_load(inst.ctrl.vec_op) && !mem::is_vec_store(inst.ctrl.vec_op),
        "execute_vec_op_on called with memory op — use element_accesses instead"
    );

    check_vill(inst.bits, vtype_bits, elen)?;
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    check_widening_lmul(inst.bits, inst.ctrl.vec_op, vtype.vlmul)?;

    let mut ctx = build_ctx_from_csrs(vtype_bits, vl, vstart, vxrm, frm, elen, zvfh);
    ctx.vm = inst.ctrl.vm;
    let operand1 = build_operand1(inst);
    let vec_op = inst.ctrl.vec_op;

    if fpu::is_vec_fp(vec_op) {
        let result = fpu::vec_fp_execute(vec_op, vpr, inst.ctrl.vd, inst.ctrl.vs2, operand1, &ctx);
        return Ok(VecExecResult { vxsat: false, ..result });
    }

    if reduction::is_reduction(vec_op) {
        let operand1_ref = VecOperand::Vector(inst.ctrl.vs1);
        let result =
            reduction::vec_reduce(vec_op, vpr, inst.ctrl.vd, inst.ctrl.vs2, &operand1_ref, &ctx);
        return Ok(VecExecResult { vxsat: false, ..result });
    }

    if mask::is_mask_op(vec_op) {
        let result =
            mask::vec_mask_execute(vec_op, vpr, inst.ctrl.vd, inst.ctrl.vs2, &operand1, &ctx);
        return Ok(VecExecResult {
            scalar_result: result.scalar_result,
            ..VecExecResult::default()
        });
    }

    if permute::is_permute(vec_op) {
        let result =
            permute::vec_permute_execute(vec_op, vpr, inst.ctrl.vd, inst.ctrl.vs2, &operand1, &ctx);
        return Ok(VecExecResult {
            scalar_result: result.scalar_result,
            ..VecExecResult::default()
        });
    }

    if crypto::is_crypto(vec_op) {
        crypto::execute_crypto(
            vec_op,
            vpr,
            inst.ctrl.vd,
            inst.ctrl.vs2,
            inst.ctrl.vs1,
            ctx.vstart,
            ctx.vl,
            inst.bits,
            inst.ctrl.vec_broadcast_vs2,
        );
        return Ok(VecExecResult::default());
    }

    let result = vec_execute(
        vec_op,
        vpr,
        inst.ctrl.vd,
        inst.ctrl.vs2,
        operand1,
        ctx.sew,
        ctx.vl,
        ctx.vstart,
        ctx.vma,
        ctx.vta,
        ctx.vlmul,
        inst.ctrl.vm,
        ctx.vxrm,
    );

    Ok(VecExecResult { fp_flags: FpFlags::NONE, ..result })
}
