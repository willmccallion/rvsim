//! RISC-V Double-Precision (D) Function Codes (funct7).
//!
//! The `funct7` field (bits 31-25) encodes the operation and format.
//! For Double Precision, the format bits (26-25) are `01`.

/// Floating-point Add (Double).
pub const FADD_D: u32 = 0b0000001;
/// Floating-point Subtract (Double).
pub const FSUB_D: u32 = 0b0000101;
/// Floating-point Multiply (Double).
pub const FMUL_D: u32 = 0b0001001;
/// Floating-point Divide (Double).
pub const FDIV_D: u32 = 0b0001101;
/// Floating-point Square Root (Double).
pub const FSQRT_D: u32 = 0b0101101;
/// Floating-point Sign Injection (Double).
pub const FSGNJ_D: u32 = 0b0010001;
/// Floating-point Min/Max (Double).
pub const FMIN_MAX_D: u32 = 0b0010101;
/// Floating-point Compare (Double).
pub const FCMP_D: u32 = 0b1010001;
/// Floating-point Classify / Move to Integer (Double).
pub const FCLASS_MV_X_D: u32 = 0b1110001;
/// Convert Integer to Double.
pub const FCVT_D_W: u32 = 0b1101001;
/// Convert Double to Integer.
pub const FCVT_W_D: u32 = 0b1100001;
/// Move Integer to Double.
pub const FMV_D_X: u32 = 0b1111001;
/// Convert Single to Double.
pub const FCVT_S_D: u32 = 0b0100000;
