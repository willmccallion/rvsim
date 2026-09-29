//! RISC-V Half-Precision (Zfh) Function Codes (funct7).
//!
//! The `funct7` field (bits 31-25) encodes the operation and format.
//! For Half Precision, the format bits (26-25) are `10`.

/// Floating-point Add (Half).
pub const FADD_H: u32 = 0b0000010;
/// Floating-point Subtract (Half).
pub const FSUB_H: u32 = 0b0000110;
/// Floating-point Multiply (Half).
pub const FMUL_H: u32 = 0b0001010;
/// Floating-point Divide (Half).
pub const FDIV_H: u32 = 0b0001110;
/// Floating-point Square Root (Half).
pub const FSQRT_H: u32 = 0b0101110;
/// Floating-point Sign Injection (Half).
pub const FSGNJ_H: u32 = 0b0010010;
/// Floating-point Min/Max (Half).
pub const FMIN_MAX_H: u32 = 0b0010110;
/// Floating-point Compare (Half).
pub const FCMP_H: u32 = 0b1010010;
/// Floating-point Classify / Move to Integer (Half).
pub const FCLASS_MV_X_H: u32 = 0b1110010;
/// Convert Float to Integer (Half).
pub const FCVT_W_H: u32 = 0b1100010;
/// Convert Integer to Float (Half).
pub const FCVT_H_W: u32 = 0b1101010;
/// Move Integer to Half.
pub const FMV_H_X: u32 = 0b1111010;
/// Convert to Half from another floating-point format.
/// Target fmt is half (10); rs2 encodes the source format.
pub const FCVT_H_FP: u32 = 0b0100010;
