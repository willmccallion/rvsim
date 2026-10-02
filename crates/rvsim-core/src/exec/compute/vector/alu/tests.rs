//! Vector integer ALU tests.

use super::*;
use crate::arch::regs::vpr::Vpr;
use crate::exec::compute::vector::context::{VecExecResult, VecOperand};
use crate::isa::op::{VecAluOp, VecClass, VectorOp};
use crate::isa::rvv::{ElemIdx, Vlen};

/// Helper: create a 128-bit VLEN VPR.
fn make_vpr() -> Vpr {
    Vpr::new(Vlen::new_unchecked(128))
}

/// The ALU operation `op` decodes to.
fn alu(op: VectorOp) -> VecAluOp {
    match op.class() {
        VecClass::Alu(op) => op,
        other => panic!("{op:?} is not an ALU op: {other:?}"),
    }
}

/// Helper: execute with common defaults (LMUL=1, unmasked, vstart=0,
/// undisturbed policies).
fn run(
    op: VectorOp,
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: VecOperand,
    sew: Sew,
    vl: usize,
) -> VecExecResult {
    vec_execute(
        alu(op),
        vpr,
        vd,
        vs2,
        operand1,
        sew,
        vl,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    )
}

#[test]
fn test_vadd_e8() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    // Write 100 to vs2[0], operate with scalar 55
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 100);
    let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(55), Sew::E8, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 155);
}

/// vwmaccus.vx: vd[i] += unsigned(rs1) * signed(vs2[i]).
/// vs2[0] = 0x80 (i8 = -128); rs1 = 0xfffffffffffffff8 (low 8 bits 0xf8 = u8 248).
/// Product = 248 * -128 = -31744 → 0x8400 in u16.
#[test]
fn test_vwmaccus_vx_signed_vs2() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0x80);
    let rs1: u64 = (-8_i64) as u64;
    let _ = run(VectorOp::VWMaccUS, &mut vpr, vd, vs2, VecOperand::Scalar(rs1), Sew::E8, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x8400);
}

/// vwmaccsu.vv: vd[i] += signed(vs1[i]) * unsigned(vs2[i]).
/// vs1[0] = 0x80 (i8 = -128), vs2[0] = 0x80 (u8 = 128). Product = -16384 = 0xc000.
#[test]
fn test_vwmaccsu_vv_signed_unsigned() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs1 = VRegIdx::new(2);
    let vs2 = VRegIdx::new(3);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0x80);
    vpr.write_element(vs1, ElemIdx::new(0), Sew::E8, 0x80);
    let _ = run(VectorOp::VWMaccSU, &mut vpr, vd, vs2, VecOperand::Vector(vs1), Sew::E8, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xc000);
}

#[test]
fn test_vadd_e16() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 1000);
    let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(2345), Sew::E16, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 3345);
}

#[test]
fn test_vadd_e32() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0x8000_0000);
    let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(0x8000_0000), Sew::E32, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0);
}

#[test]
fn test_vadd_e64() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E64, 0xFFFF_FFFF_FFFF_FFFE);
    let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Scalar(3), Sew::E64, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E64), 1);
}

#[test]
fn test_vadd_vv_multiple_elements() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(3);
    let vs2 = VRegIdx::new(4);
    let vs1 = VRegIdx::new(5);
    // VLEN=128, SEW=32 → 4 elements per register
    for i in 0..4 {
        vpr.write_element(vs2, ElemIdx::new(i), Sew::E32, (i as u64) * 10);
        vpr.write_element(vs1, ElemIdx::new(i), Sew::E32, (i as u64) + 1);
    }
    let _ = run(VectorOp::VAdd, &mut vpr, vd, vs2, VecOperand::Vector(vs1), Sew::E32, 4);
    for i in 0..4 {
        let expected = (i as u64) * 10 + (i as u64) + 1;
        assert_eq!(vpr.read_element(vd, ElemIdx::new(i), Sew::E32), expected);
    }
}

#[test]
fn test_vmslt_signed() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    // Write -1 (0xFF) to vs2[0] at E8, compare with 1
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
    let _ = run(VectorOp::VMSlt, &mut vpr, vd, vs2, VecOperand::Scalar(1), Sew::E8, 1);
    // -1 < 1 should be true
    assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
}

#[test]
fn test_vmseq() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
    vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 43);
    let _ = run(VectorOp::VMSeq, &mut vpr, vd, vs2, VecOperand::Scalar(42), Sew::E32, 2);
    assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
    assert!(!vpr.read_mask_bit(vd, ElemIdx::new(1)));
}

#[test]
fn test_vdivu_by_zero() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
    let _ = run(VectorOp::VDivU, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E32, 1);
    // div by zero → all-1s
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xFFFF_FFFF);
}

#[test]
fn test_vdiv_by_zero() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 100);
    let _ = run(VectorOp::VDiv, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E16, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xFFFF);
}

#[test]
fn test_vremu_by_zero() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 42);
    let _ = run(VectorOp::VRemU, &mut vpr, vd, vs2, VecOperand::Scalar(0), Sew::E32, 1);
    // rem by zero → dividend
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 42);
}

#[test]
fn test_vdiv_signed_overflow() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    // MIN_INT(E32) = 0x80000000, -1 = 0xFFFFFFFF
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0x8000_0000);
    let _ = run(VectorOp::VDiv, &mut vpr, vd, vs2, VecOperand::Scalar(0xFFFF_FFFF), Sew::E32, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0x8000_0000);
}

#[test]
fn test_vwaddu() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(2);
    let vs2 = VRegIdx::new(4);
    // E16 + E16 → E32
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 0xFFFF);
    let res = vec_execute(
        alu(VectorOp::VWAddU),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(1),
        Sew::E16,
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );
    assert!(!res.vxsat);
    // 0xFFFF + 1 = 0x10000 (doesn't overflow because result is E32)
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0x10000);
}

#[test]
fn test_vwaddu_mf8_widen() {
    // vwaddu.vv at SEW=E8, LMUL=mf8 with vl=2.
    // VLMAX = (128/8)*1/8 = 2 elements at E8. Widens to E16.
    // Source elements vs2[0]=5, vs2[1]=7. Scalar = 3.
    // Expected: vd[0]@E16 = 8, vd[1]@E16 = 10. Tail bytes unchanged (tu).
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(4);
    let vs2 = VRegIdx::new(8);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 5);
    vpr.write_element(vs2, ElemIdx::new(1), Sew::E8, 7);
    // Sentinel: pre-fill v4 with 0xAA to detect tail clobbering.
    for i in 0..16usize {
        vpr.write_element(vd, ElemIdx::new(i), Sew::E8, 0xAA);
    }

    let _ = vec_execute(
        alu(VectorOp::VWAddU),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(3),
        Sew::E8,
        2,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::Mf8,
        true,
        Vxrm::RoundToNearestUp,
    );

    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x0008);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E16), 0x000a);
    // Tail bytes [4..16] should still be 0xAA (Undisturbed)
    for i in 4..16 {
        assert_eq!(vpr.read_element(vd, ElemIdx::new(i), Sew::E8), 0xAA, "tail byte {i} clobbered");
    }
}

#[test]
fn test_vwadd_signed() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(2);
    let vs2 = VRegIdx::new(4);
    // -1 at E8 (0xFF) + -2 at E8 (0xFE) → -3 at E16 (0xFFFD)
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
    let _ = vec_execute(
        alu(VectorOp::VWAdd),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(0xFE),
        Sew::E8,
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );
    let result = vpr.read_element(vd, ElemIdx::new(0), Sew::E16);
    // sign_extend(0xFF, E8) = -1, sign_extend(0xFE, E8) = -2, sum = -3
    // -3 as u16 = 0xFFFD
    assert_eq!(result, 0xFFFD);
}

#[test]
fn test_vsaddu_saturation() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 200);
    let res = run(VectorOp::VSAddU, &mut vpr, vd, vs2, VecOperand::Scalar(100), Sew::E8, 1);
    // 200 + 100 = 300, saturates to 255
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 0xFF);
    assert!(res.vxsat);
}

#[test]
fn test_vsaddu_no_saturation() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 100);
    let res = run(VectorOp::VSAddU, &mut vpr, vd, vs2, VecOperand::Scalar(50), Sew::E8, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 150);
    assert!(!res.vxsat);
}

#[test]
fn test_masked_operation() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    let v0 = VRegIdx::new(0);

    // Set up mask: element 0 active, element 1 inactive
    vpr.write_mask_bit(v0, ElemIdx::new(0), true);
    vpr.write_mask_bit(v0, ElemIdx::new(1), false);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
    vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 20);

    // Pre-fill vd with sentinel values
    vpr.write_element(vd, ElemIdx::new(0), Sew::E32, 0xDEAD);
    vpr.write_element(vd, ElemIdx::new(1), Sew::E32, 0xBEEF);

    let _ = vec_execute(
        alu(VectorOp::VAdd),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(5),
        Sew::E32,
        2,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        false, // masked
        Vxrm::RoundToNearestUp,
    );

    // Element 0 is active: 10 + 5 = 15
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 15);
    // Element 1 is inactive with undisturbed policy: keep 0xBEEF
    assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xBEEF);
}

#[test]
fn test_tail_agnostic() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
    // Pre-fill tail element
    vpr.write_element(vd, ElemIdx::new(1), Sew::E32, 0x1234);

    let _ = vec_execute(
        alu(VectorOp::VAdd),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(5),
        Sew::E32,
        1, // vl=1, so element 1 is tail
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Agnostic,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );

    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 15);
    // Tail element with agnostic: written with all-1s per RVV 1.0
    assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), Sew::E32.ones());
}

#[test]
fn test_vmerge() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(3);
    let vs2 = VRegIdx::new(4);
    let v0 = VRegIdx::new(0);

    vpr.write_mask_bit(v0, ElemIdx::new(0), false);
    vpr.write_mask_bit(v0, ElemIdx::new(1), true);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 0xAAAA);
    vpr.write_element(vs2, ElemIdx::new(1), Sew::E32, 0xBBBB);

    let _ = vec_execute(
        alu(VectorOp::VMerge),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(0xCCCC),
        Sew::E32,
        2,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        false, // masked merge
        Vxrm::RoundToNearestUp,
    );

    // Element 0: mask bit=0 → take vs2 = 0xAAAA
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0xAAAA);
    // Element 1: mask bit=1 → take operand1 = 0xCCCC
    assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xCCCC);
}

#[test]
fn test_vmacc() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 7);
    vpr.write_element(vd, ElemIdx::new(0), Sew::E32, 100);

    // vmacc: vd = vs1 * vs2 + vd = 3 * 7 + 100 = 121
    let _ = run(VectorOp::VMacc, &mut vpr, vd, vs2, VecOperand::Scalar(3), Sew::E32, 1);
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 121);
}

#[test]
fn test_vsext_vf2() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);

    // Write -1 as E8 (0xFF), sign-extend to E16 should be 0xFFFF
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
    let _ = vec_execute(
        alu(VectorOp::VSextVf2),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(0), // unused for extension ops
        Sew::E16,
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0xFFFF);
}

#[test]
fn test_vzext_vf2() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E8, 0xFF);
    let _ = vec_execute(
        alu(VectorOp::VZextVf2),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(0),
        Sew::E16,
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E16), 0x00FF);
}

#[test]
fn test_vadc() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);
    let v0 = VRegIdx::new(0);

    vpr.write_element(vs2, ElemIdx::new(0), Sew::E32, 10);
    vpr.write_mask_bit(v0, ElemIdx::new(0), true); // carry = 1

    let _ = vec_execute(
        alu(VectorOp::VAdc),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(20),
        Sew::E32,
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        false, // use v0 as carry
        Vxrm::RoundToNearestUp,
    );

    // 10 + 20 + 1 (carry) = 31
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 31);
}

#[test]
fn test_vnsrl() {
    let mut vpr = make_vpr();
    let vd = VRegIdx::new(1);
    let vs2 = VRegIdx::new(2);

    // Write 0x1234 at E16, narrow to E8 with shift right by 8
    vpr.write_element(vs2, ElemIdx::new(0), Sew::E16, 0x1234);
    let _ = vec_execute(
        alu(VectorOp::VNSrl),
        &mut vpr,
        vd,
        vs2,
        VecOperand::Scalar(8),
        Sew::E8, // destination SEW
        1,
        0,
        MaskPolicy::Undisturbed,
        TailPolicy::Undisturbed,
        Vlmul::M1,
        true,
        Vxrm::RoundToNearestUp,
    );
    assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 0x12);
}
