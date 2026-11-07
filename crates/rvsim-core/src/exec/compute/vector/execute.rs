//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR or `VecPrfView`).
//!
//! `execute_vec_op_on()` writes results to a `&mut impl VectorRegFile` (a
//! `VecPrfView` on the O3 backend, a `ShadowVpr` on the in-order one) and
//! returns the side effects for commit-time application.

use crate::exec::compute::vector::alu::vec_execute;
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand, widen_sew};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::exec::compute::vector::{crypto, fpu, mask, permute, reduction};
use crate::exec::inst::Inst;
use crate::isa::encoding::rvv::encoding as v_enc;
use crate::isa::op::{VecClass, VecSrcEncoding, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{Sew, VectorConfig, VectorRounding, Vlmul, parse_vtype_with_elen};

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
                    // Gather takes a zero-extended 5-bit immediate, not a
                    // sign-extended one.
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
const fn check_vill(inst: &Inst, vtype_bits: u64, elen: usize) -> Result<(), Trap> {
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    if vtype.vill && inst.ctrl.vec_op.depends_on_vtype() {
        return Err(Trap::IllegalInstruction(inst.bits));
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

/// Whether the unit computes in every floating-point width `op` uses at
/// element width `sew`: f16 needs Zvfh and f64 an ELEN of 64, there is no
/// f8, and an integer operand twice SEW wide must fit in ELEN.
fn fp_widths_supported(op: VectorOp, sew: Sew, zvfh: bool, elen: usize) -> bool {
    let fp = |width: Option<Sew>| match width {
        Some(Sew::E16) => zvfh,
        Some(Sew::E32) => true,
        Some(Sew::E64) => elen >= 64,
        Some(Sew::E8) | None => false,
    };
    let int = |width: Option<Sew>| width.is_some_and(|w| w.bits() <= elen);
    let wide = widen_sew(sew);
    match op {
        VectorOp::VFWCvtXuF
        | VectorOp::VFWCvtXF
        | VectorOp::VFWCvtRtzXuF
        | VectorOp::VFWCvtRtzXF
        | VectorOp::VFNCvtFXu
        | VectorOp::VFNCvtFX => fp(Some(sew)) && int(wide),
        VectorOp::VFWCvtFXu
        | VectorOp::VFWCvtFX
        | VectorOp::VFNCvtXuF
        | VectorOp::VFNCvtXF
        | VectorOp::VFNCvtRtzXuF
        | VectorOp::VFNCvtRtzXF => fp(wide),
        VectorOp::VFWAdd
        | VectorOp::VFWSub
        | VectorOp::VFWMul
        | VectorOp::VFWAddW
        | VectorOp::VFWSubW
        | VectorOp::VFWMacc
        | VectorOp::VFWNMacc
        | VectorOp::VFWMSac
        | VectorOp::VFWNMSac
        | VectorOp::VFWCvtFF
        | VectorOp::VFWRedOSum
        | VectorOp::VFWRedUSum
        | VectorOp::VFNCvtFF
        | VectorOp::VFNCvtRodFF => fp(Some(sew)) && fp(wide),
        VectorOp::VFRedMax | VectorOp::VFRedMin | VectorOp::VFRedOSum | VectorOp::VFRedUSum => {
            fp(Some(sew))
        }
        _ if fpu::is_vec_fp(op) => fp(Some(sew)),
        _ => true,
    }
}

/// Build execution context from the CSR snapshot (no `SystemState` reference needed).
const fn build_ctx_from_csrs(
    config: VectorConfig,
    rounding: VectorRounding,
    elen: usize,
    zvfh: bool,
) -> VecExecCtx {
    let vtype = parse_vtype_with_elen(config.vtype, elen);
    VecExecCtx {
        sew: vtype.vsew,
        vl: config.vl as usize,
        vstart: config.vstart as usize,
        vma: vtype.vma,
        vta: vtype.vta,
        vlmul: vtype.vlmul,
        vm: true, // overridden per-instruction
        vxrm: rounding.vxrm,
        frm: rounding.frm,
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
pub fn execute_vec_op_on<V: VectorRegFile>(
    vpr: &mut V,
    config: VectorConfig,
    rounding: VectorRounding,
    elen: usize,
    zvfh: bool,
    inst: &Inst,
) -> Result<VecExecResult, Trap> {
    check_vill(inst, config.vtype, elen)?;
    let vtype = parse_vtype_with_elen(config.vtype, elen);
    check_widening_lmul(inst.bits, inst.ctrl.vec_op, vtype.vlmul)?;
    if !fp_widths_supported(inst.ctrl.vec_op, vtype.vsew, zvfh, elen) {
        return Err(Trap::IllegalInstruction(inst.bits));
    }

    let mut ctx = build_ctx_from_csrs(config, rounding, elen, zvfh);
    ctx.vm = inst.ctrl.vm;
    if ctx.vstart >= ctx.vl && !inst.ctrl.vec_op.executes_without_body() {
        return Ok(VecExecResult::default());
    }
    let operand1 = build_operand1(inst);
    let vec_op = inst.ctrl.vec_op;
    let (vd, vs2, vs1) = (inst.ctrl.vd, inst.ctrl.vs2, inst.ctrl.vs1);

    let result = match vec_op.class() {
        VecClass::Fp => fpu::vec_fp_execute(vec_op, vpr, vd, vs2, operand1, &ctx),
        VecClass::Reduce(op) => reduction::vec_reduce(op, vpr, vd, vs2, vs1, &ctx),
        VecClass::Mask(op) => {
            let result = mask::vec_mask_execute(op, vpr, vd, vs2, vs1, &ctx);
            VecExecResult { scalar_result: result.scalar_result, ..VecExecResult::default() }
        }
        VecClass::Permute(op) => {
            let sources = permute::PermuteSources { operand1, vs1, rs1: inst.rv1 };
            let result = permute::vec_permute_execute(op, vpr, vd, vs2, sources, &ctx);
            VecExecResult { scalar_result: result.scalar_result, ..VecExecResult::default() }
        }
        VecClass::Crypto(op) => {
            crypto::execute_crypto(
                op,
                vpr,
                vd,
                vs2,
                vs1,
                ctx.vstart,
                ctx.vl,
                inst.bits,
                inst.ctrl.vec_broadcast_vs2,
            );
            VecExecResult::default()
        }
        VecClass::Alu(op) => vec_execute(op, vpr, vd, vs2, operand1, &ctx),
        VecClass::None | VecClass::Config | VecClass::Load | VecClass::Store => {
            debug_assert!(false, "execute_vec_op_on called with a config or memory op");
            VecExecResult::default()
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::regs::vpr::Vpr;
    use crate::exec::signals::ControlSignals;
    use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlen};

    const E32_M1_TAIL_AGNOSTIC: u64 = 0x50;
    const ELEN: usize = 64;
    const VD_BEFORE: u64 = 0x1234_5678;

    fn vector_inst(vec_op: VectorOp) -> Inst {
        let ctrl = ControlSignals {
            vec_op,
            vd: VRegIdx::new(1),
            vs2: VRegIdx::new(2),
            vs1: VRegIdx::new(3),
            vm: true,
            ..ControlSignals::default()
        };
        Inst { ctrl, ..Inst::default() }
    }

    /// Registers with `VD_BEFORE` in every element of `v1`, `7` in `v2[0]`
    /// and `v3[0]`.
    fn registers() -> Vpr {
        let mut vpr = Vpr::new(Vlen::new_unchecked(128));
        for i in 0..4 {
            vpr.write_element(VRegIdx::new(1), ElemIdx::new(i), Sew::E32, VD_BEFORE);
        }
        vpr.write_element(VRegIdx::new(2), ElemIdx::new(0), Sew::E32, 7);
        vpr.write_element(VRegIdx::new(3), ElemIdx::new(0), Sew::E32, 7);
        vpr
    }

    fn run(vpr: &mut Vpr, vec_op: VectorOp, vl: u64, vstart: u64) -> VecExecResult {
        let inst = vector_inst(vec_op);
        let config = VectorConfig { vtype: E32_M1_TAIL_AGNOSTIC, vl, vstart };
        execute_vec_op_on(vpr, config, VectorRounding::default(), ELEN, false, &inst).unwrap()
    }

    fn vd_elements(vpr: &Vpr) -> Vec<u64> {
        (0..4).map(|i| vpr.read_element(VRegIdx::new(1), ElemIdx::new(i), Sew::E32)).collect()
    }

    #[test]
    fn a_reduction_with_vl_zero_leaves_its_destination_alone() {
        let mut vpr = registers();

        let _result = run(&mut vpr, VectorOp::VRedSum, 0, 0);

        assert_eq!(vd_elements(&vpr), vec![VD_BEFORE; 4]);
    }

    #[test]
    fn a_tail_agnostic_op_with_vstart_at_vl_fills_no_tail_element() {
        let mut vpr = registers();

        let _result = run(&mut vpr, VectorOp::VAdd, 2, 2);

        assert_eq!(vd_elements(&vpr), vec![VD_BEFORE; 4]);
    }

    #[test]
    fn vmv_x_s_reads_element_zero_when_vl_is_zero() {
        let mut vpr = registers();

        let result = run(&mut vpr, VectorOp::VMvXS, 0, 0);

        assert_eq!(result.scalar_result, Some(7));
    }

    #[test]
    fn a_whole_register_move_copies_when_vl_is_zero() {
        let mut vpr = registers();

        let _result = run(&mut vpr, VectorOp::VMv1r, 0, 0);

        assert_eq!(vd_elements(&vpr), vec![7, 0, 0, 0]);
    }

    /// Whether `vec_op` executes at element width `sew`, rather than
    /// raising illegal-instruction, on a unit with `zvfh` and `elen`.
    fn legal(vec_op: VectorOp, sew: Sew, zvfh: bool, elen: usize) -> bool {
        let mut vpr = registers();
        let vtype = u64::from(sew.to_encoding()) << 3;
        let inst = vector_inst(vec_op);
        let config = VectorConfig { vtype, vl: 1, vstart: 0 };
        execute_vec_op_on(&mut vpr, config, VectorRounding::default(), elen, zvfh, &inst).is_ok()
    }

    #[test]
    fn floating_point_arithmetic_at_sew_8_is_illegal() {
        assert!(!legal(VectorOp::VFAdd, Sew::E8, true, ELEN));
    }

    #[test]
    fn half_precision_arithmetic_needs_zvfh() {
        let legality = [false, true].map(|zvfh| legal(VectorOp::VFAdd, Sew::E16, zvfh, ELEN));

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn an_8_bit_integer_widens_to_half_precision_with_zvfh() {
        let legality = [false, true].map(|zvfh| legal(VectorOp::VFWCvtFX, Sew::E8, zvfh, ELEN));

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn half_precision_narrows_to_an_8_bit_integer_with_zvfh() {
        let legality = [false, true].map(|zvfh| legal(VectorOp::VFNCvtXF, Sew::E8, zvfh, ELEN));

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn a_16_bit_integer_widens_to_single_precision_without_zvfh() {
        assert!(legal(VectorOp::VFWCvtFX, Sew::E16, false, ELEN));
    }

    #[test]
    fn widening_single_precision_needs_an_elen_of_64() {
        let legality = [32, 64].map(|elen| legal(VectorOp::VFWAdd, Sew::E32, true, elen));

        assert_eq!(legality, [false, true], "[ELEN 32, ELEN 64]");
    }
}
