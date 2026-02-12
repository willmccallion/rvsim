//! RISC-V Floating-Point (F/D) Function Codes (funct3).
//!
//! These values are used in the `funct3` field to distinguish between
//! variants of floating-point instructions (e.g., comparison types,
//! sign injection modes, or rounding modes).

/// Floating-point Sign Injection (Copy sign).
pub const FSGNJ: u32 = 0b000;
/// Floating-point Sign Injection Negate (Negate sign).
pub const FSGNJN: u32 = 0b001;
/// Floating-point Sign Injection XOR (XOR sign).
pub const FSGNJX: u32 = 0b010;

/// Floating-point Minimum.
pub const FMIN: u32 = 0b000;
/// Floating-point Maximum.
pub const FMAX: u32 = 0b001;

/// Floating-point Equal (FEQ).
pub const FEQ: u32 = 0b000;
/// Floating-point Less Than (FLT).
pub const FLT: u32 = 0b001;
/// Floating-point Less Than or Equal (FLE).
pub const FLE: u32 = 0b010;

/// Floating-point Classify (FCLASS).
pub const FCLASS: u32 = 0b001;
/// Move Integer to Floating-Point (FMV.W.X / FMV.D.X).
pub const FMV_X_W: u32 = 0b000;
