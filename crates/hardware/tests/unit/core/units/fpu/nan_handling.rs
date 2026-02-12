use riscv_core::core::pipeline::signals::AluOp;
use riscv_core::core::units::fpu::Fpu;

#[test]
fn test_box_f32() {
    let f: f32 = 1.234;
    let boxed = Fpu::box_f32(f);
    // RISC-V: 32-bit values are boxed in 64-bit registers by setting upper 32 bits to all 1s.
    assert_eq!(boxed >> 32, 0xFFFFFFFF, "Upper 32 bits must be all 1s");
    assert_eq!(
        boxed as u32,
        f.to_bits(),
        "Lower 32 bits must match f32 representation"
    );
}

#[test]
fn test_nan_boxing_unboxing() {
    // Valid boxing
    let f_val: f32 = 42.0;
    let valid_boxed = Fpu::box_f32(f_val);

    // We'll test this through an operation like FAdd with 0
    let zero = Fpu::box_f32(0.0);
    let res = Fpu::execute(AluOp::FAdd, valid_boxed, zero, 0, true);

    assert_eq!(res >> 32, 0xFFFFFFFF);
    assert_eq!(f32::from_bits(res as u32), 42.0);

    // Invalid boxing (upper bits not all 1s)
    let invalid_boxed = (f_val.to_bits() as u64) | 0x00000000_00000000; // Upper bits are 0
    let res_invalid = Fpu::execute(AluOp::FAdd, invalid_boxed, zero, 0, true);

    // RISC-V: If input is not properly NaN-boxed, it is treated as canonical NaN.
    let canon_nan_32 = 0x7fc00000u32;
    assert_eq!(
        res_invalid,
        Fpu::box_f32(f32::from_bits(canon_nan_32)),
        "Invalidly boxed input must result in canonical NaN"
    );
}

#[test]
fn test_fmin_fmax_nan_handling() {
    let f_val = Fpu::box_f32(10.0);
    let f_nan = Fpu::box_f32(f32::NAN);

    // RISC-V fmin/fmax: if one op is NaN, return the other.
    let res_min = Fpu::execute(AluOp::FMin, f_val, f_nan, 0, true);
    assert_eq!(res_min, f_val, "fmin(val, NaN) should be val");

    let res_min2 = Fpu::execute(AluOp::FMin, f_nan, f_val, 0, true);
    assert_eq!(res_min2, f_val, "fmin(NaN, val) should be val");

    let res_max = Fpu::execute(AluOp::FMax, f_val, f_nan, 0, true);
    assert_eq!(res_max, f_val, "fmax(val, NaN) should be val");

    let res_max2 = Fpu::execute(AluOp::FMax, f_nan, f_val, 0, true);
    assert_eq!(res_max2, f_val, "fmax(NaN, val) should be val");

    // Both NaN: return canonical NaN
    let res_both_nan = Fpu::execute(AluOp::FMin, f_nan, f_nan, 0, true);
    let canon_nan_32 = 0x7fc00000u32;
    assert_eq!(res_both_nan, Fpu::box_f32(f32::from_bits(canon_nan_32)));
}

#[test]
fn test_canonical_nan_propagation() {
    // Signaling NaN (sNaN)
    let snan_bits = 0x7f800001u32;
    let snan = Fpu::box_f32(f32::from_bits(snan_bits));
    let zero = Fpu::box_f32(0.0);

    // Any op with sNaN should produce a canonical quiet NaN
    let res = Fpu::execute(AluOp::FAdd, snan, zero, 0, true);
    let canon_nan_32 = 0x7fc00000u32;
    assert_eq!(
        res,
        Fpu::box_f32(f32::from_bits(canon_nan_32)),
        "sNaN must be quieted to canonical NaN"
    );
}

#[test]
fn test_f64_nan_boxing_not_applicable() {
    // NaN boxing only applies to 32-bit values in 64-bit registers.
    // 64-bit operations should use the full 64 bits.
    let d_val1 = f64::to_bits(1.0);
    let d_val2 = f64::to_bits(2.0);

    let res = Fpu::execute(AluOp::FAdd, d_val1, d_val2, 0, false);
    assert_eq!(f64::from_bits(res), 3.0);
}
