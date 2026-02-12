//! RISC-V Single-Precision (F) Function Codes (funct7).
//!
//! The `funct7` field (bits 31-25) encodes the operation and format.
//! For Single Precision, the format bits (26-25) are `00`.

/// Floating-point Add (Single).
pub const FADD: u32 = 0b0000000;
/// Floating-point Subtract (Single).
pub const FSUB: u32 = 0b0000100;
/// Floating-point Multiply (Single).
pub const FMUL: u32 = 0b0001000;
/// Floating-point Divide (Single).
pub const FDIV: u32 = 0b0001100;
/// Floating-point Square Root (Single).
pub const FSQRT: u32 = 0b0101100;
/// Floating-point Sign Injection (Single).
pub const FSGNJ: u32 = 0b0010000;
/// Floating-point Min/Max (Single).
pub const FMIN_MAX: u32 = 0b0010100;
/// Floating-point Compare (Single).
pub const FCMP: u32 = 0b1010000;
/// Floating-point Classify / Move to Integer (Single).
pub const FCLASS_MV_X_F: u32 = 0b1110000;
/// Convert Integer to Float (Single).
pub const FCVT_W_F: u32 = 0b1100000;
/// Convert Float to Integer (Single).
pub const FCVT_F_W: u32 = 0b1101000;
/// Move Float to Integer (Single).
pub const FMV_F_X: u32 = 0b1111000;
/// Convert Double to Single.
pub const FCVT_DS: u32 = 0b0100001;
