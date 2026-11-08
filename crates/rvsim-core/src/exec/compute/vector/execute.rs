//! Vector instruction execution dispatch.
//!
//! Bridges the pipeline (which carries scalar values in latches) to the VPU
//! execution modules (which operate on the architectural VPR or `VecPrfView`).
//!
//! `execute_vec_op_on()` writes results to a `&mut impl VectorRegFile` (a
//! `VecPrfView` on the O3 backend, a `ShadowVpr` on the in-order one) and
//! returns the side effects for commit-time application.

use crate::exec::compute::vector::alu::vec_execute;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, VecSources, widen_sew,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::exec::compute::vector::{crypto, fpu, mask, permute, reduction};
use crate::exec::inst::Inst;
use crate::isa::op::{
    FpNarrowConvertOp, FpWidenConvertOp, IntConvert, ReduceOp, VecAluOp, VecFpOp, VecSrcEncoding,
    VectorCompute, VectorOp,
};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{Sew, VectorConfig, VectorRounding, Vlmul, parse_vtype_with_elen};

/// The first source operand: `vs1`, the scalar `rs1`/`fs1` value, or the
/// immediate decode extracted.
const fn build_operand1(inst: &Inst) -> VecOperand {
    match inst.ctrl.vec_src_encoding {
        VecSrcEncoding::VV => VecOperand::Vector(inst.ctrl.vs1),
        VecSrcEncoding::VX | VecSrcEncoding::VF => VecOperand::Scalar(inst.rv1),
        VecSrcEncoding::VI(imm) => VecOperand::Immediate(imm as i64),
        VecSrcEncoding::None => VecOperand::Scalar(0),
    }
}

/// Check vill and return `IllegalInstruction` trap if set.
///
/// RVV 1.0 §3.3: "If the vill bit is set, then any attempt to execute a
/// vector instruction that depends on vtype will raise an illegal-instruction
/// exception."
#[inline]
const fn check_vill(op: VectorOp, inst: u32, vtype_bits: u64, elen: usize) -> Result<(), Trap> {
    let vtype = parse_vtype_with_elen(vtype_bits, elen);
    if vtype.vill && op.depends_on_vtype() {
        return Err(Trap::IllegalInstruction(inst));
    }
    Ok(())
}

/// Returns `true` if `op` widens (or reads/writes a 2×SEW operand) and would
/// therefore require `EMUL = 2 × LMUL`.
const fn op_uses_widened_emul(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::Alu(VecAluOp::Widen(_) | VecAluOp::WidenMacc(_) | VecAluOp::Narrow(_))
            | VectorOp::Fp(
                VecFpOp::Widen(_)
                    | VecFpOp::WidenFma(_)
                    | VecFpOp::WidenConvert(_)
                    | VecFpOp::NarrowConvert(_)
            )
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
        VectorOp::Fp(
            VecFpOp::WidenConvert(FpWidenConvertOp::Int(IntConvert::ToInt(_, _)))
            | VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(IntConvert::FromInt(_))),
        ) => fp(Some(sew)) && int(wide),
        VectorOp::Fp(
            VecFpOp::WidenConvert(FpWidenConvertOp::Int(IntConvert::FromInt(_)))
            | VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(IntConvert::ToInt(_, _))),
        ) => fp(wide),
        VectorOp::Fp(
            VecFpOp::Widen(_)
            | VecFpOp::WidenFma(_)
            | VecFpOp::WidenConvert(FpWidenConvertOp::Float)
            | VecFpOp::NarrowConvert(FpNarrowConvertOp::Float | FpNarrowConvertOp::FloatRoundToOdd),
        )
        | VectorOp::Reduce(ReduceOp::FpWiden(_)) => fp(Some(sew)) && fp(wide),
        VectorOp::Fp(_) | VectorOp::Reduce(ReduceOp::Fp(_)) => fp(Some(sew)),
        _ => true,
    }
}

/// The vector state an instruction executes under: the CSRs it reads and
/// what the hart supports.
#[derive(Clone, Copy, Debug)]
pub struct VecEnv {
    /// `vtype`, `vl` and `vstart`.
    pub config: VectorConfig,
    /// `vxrm` and `frm`.
    pub rounding: VectorRounding,
    /// The hart's ELEN in bits.
    pub elen: usize,
    /// Whether the hart has Zvfh.
    pub zvfh: bool,
}

/// Build execution context from the CSR snapshot (no `SystemState` reference needed).
const fn build_ctx_from_csrs(env: VecEnv) -> VecExecCtx {
    let VecEnv { config, rounding, elen, zvfh } = env;
    let vtype = parse_vtype_with_elen(config.vtype(), elen);
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
    op: VectorCompute,
    inst: &Inst,
    env: VecEnv,
) -> Result<VecExecResult, Trap> {
    let vec_op = VectorOp::from(op);
    check_vill(vec_op, inst.bits, env.config.vtype(), env.elen)?;
    let vtype = parse_vtype_with_elen(env.config.vtype(), env.elen);
    check_widening_lmul(inst.bits, vec_op, vtype.vlmul)?;
    if !fp_widths_supported(vec_op, vtype.vsew, env.zvfh, env.elen) {
        return Err(Trap::IllegalInstruction(inst.bits));
    }

    let mut ctx = build_ctx_from_csrs(env);
    ctx.vm = inst.ctrl.vm;
    if ctx.vstart >= ctx.vl && !vec_op.executes_without_body() {
        return Ok(VecExecResult::default());
    }
    let operand1 = build_operand1(inst);
    let (vd, vs2, vs1) = (inst.ctrl.vd, inst.ctrl.vs2, inst.ctrl.vs1);
    let sources = VecSources { operand1, vs1, rs1: inst.rv1 };

    let result = match op {
        VectorCompute::Fp(op) => fpu::vec_fp_execute(op, vpr, vd, vs2, sources, &ctx),
        VectorCompute::Reduce(op) => reduction::vec_reduce(op, vpr, vd, vs2, vs1, &ctx),
        VectorCompute::Mask(op) => mask::vec_mask_execute(op, vpr, vd, vs2, vs1, &ctx),
        VectorCompute::Permute(op) => permute::vec_permute_execute(op, vpr, vd, vs2, sources, &ctx),
        VectorCompute::Crypto(op) => {
            crypto::execute_crypto(op, vpr, &inst.ctrl, &ctx);
            VecExecResult::default()
        }
        VectorCompute::Alu(op) => vec_execute(op, vpr, vd, vs2, operand1, &ctx),
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::regs::vpr::Vpr;
    use crate::exec::signals::ControlSignals;
    use crate::isa::op::{
        FpBinaryOp, FpWidenOp, IntOp, IntReduceOp, IntRounding, PermuteOp, Signedness,
        WholeRegisters,
    };
    use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlen};

    const E32_M1_TAIL_AGNOSTIC: u64 = 0x50;
    const ELEN: usize = 64;
    const VD_BEFORE: u64 = 0x1234_5678;

    fn vector_inst(vec_op: VectorOp) -> Inst {
        let ctrl = ControlSignals {
            vec_op: Some(vec_op),
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
        let config = VectorConfig::new(E32_M1_TAIL_AGNOSTIC, vl, vstart);
        let env = VecEnv { config, rounding: VectorRounding::default(), elen: ELEN, zvfh: false };
        execute_vec_op_on(vpr, vec_op.compute().unwrap(), &inst, env).unwrap()
    }

    fn vd_elements(vpr: &Vpr) -> Vec<u64> {
        (0..4).map(|i| vpr.read_element(VRegIdx::new(1), ElemIdx::new(i), Sew::E32)).collect()
    }

    #[test]
    fn a_reduction_with_vl_zero_leaves_its_destination_alone() {
        let mut vpr = registers();

        let _result = run(&mut vpr, VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Sum)), 0, 0);

        assert_eq!(vd_elements(&vpr), vec![VD_BEFORE; 4]);
    }

    #[test]
    fn a_tail_agnostic_op_with_vstart_at_vl_fills_no_tail_element() {
        let mut vpr = registers();

        let _result = run(&mut vpr, VectorOp::Alu(VecAluOp::Int(IntOp::Add)), 2, 2);

        assert_eq!(vd_elements(&vpr), vec![VD_BEFORE; 4]);
    }

    #[test]
    fn vmv_x_s_reads_element_zero_when_vl_is_zero() {
        let mut vpr = registers();

        let result = run(&mut vpr, VectorOp::Permute(PermuteOp::MvXS), 0, 0);

        assert_eq!(result.scalar_result, Some(7));
    }

    #[test]
    fn a_whole_register_move_copies_when_vl_is_zero() {
        let mut vpr = registers();

        let _result =
            run(&mut vpr, VectorOp::Permute(PermuteOp::WholeMove(WholeRegisters::One)), 0, 0);

        assert_eq!(vd_elements(&vpr), vec![7, 0, 0, 0]);
    }

    /// Whether `vec_op` executes at element width `sew`, rather than
    /// raising illegal-instruction, on a unit with `zvfh` and `elen`.
    fn legal(vec_op: VectorOp, sew: Sew, zvfh: bool, elen: usize) -> bool {
        let mut vpr = registers();
        let vtype = u64::from(sew.to_encoding()) << 3;
        let inst = vector_inst(vec_op);
        let config = VectorConfig::new(vtype, 1, 0);
        let env = VecEnv { config, rounding: VectorRounding::default(), elen, zvfh };
        execute_vec_op_on(&mut vpr, vec_op.compute().unwrap(), &inst, env).is_ok()
    }

    #[test]
    fn floating_point_arithmetic_at_sew_8_is_illegal() {
        assert!(!legal(VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Add)), Sew::E8, true, ELEN));
    }

    #[test]
    fn half_precision_arithmetic_needs_zvfh() {
        let legality = [false, true].map(|zvfh| {
            legal(VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Add)), Sew::E16, zvfh, ELEN)
        });

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn an_8_bit_integer_widens_to_half_precision_with_zvfh() {
        let legality = [false, true].map(|zvfh| {
            legal(
                VectorOp::Fp(VecFpOp::WidenConvert(FpWidenConvertOp::Int(IntConvert::FromInt(
                    Signedness::Signed,
                )))),
                Sew::E8,
                zvfh,
                ELEN,
            )
        });

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn half_precision_narrows_to_an_8_bit_integer_with_zvfh() {
        let legality = [false, true].map(|zvfh| {
            legal(
                VectorOp::Fp(VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(IntConvert::ToInt(
                    Signedness::Signed,
                    IntRounding::Dynamic,
                )))),
                Sew::E8,
                zvfh,
                ELEN,
            )
        });

        assert_eq!(legality, [false, true], "[without Zvfh, with Zvfh]");
    }

    #[test]
    fn a_16_bit_integer_widens_to_single_precision_without_zvfh() {
        assert!(legal(
            VectorOp::Fp(VecFpOp::WidenConvert(FpWidenConvertOp::Int(IntConvert::FromInt(
                Signedness::Signed
            )))),
            Sew::E16,
            false,
            ELEN
        ));
    }

    #[test]
    fn widening_single_precision_needs_an_elen_of_64() {
        let legality = [32, 64]
            .map(|elen| legal(VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Add)), Sew::E32, true, elen));

        assert_eq!(legality, [false, true], "[ELEN 32, ELEN 64]");
    }
}
