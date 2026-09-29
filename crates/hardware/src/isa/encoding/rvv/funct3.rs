//! Vector arithmetic funct3 categories (bits 14:12 of OP-V).

/// Integer vector-vector.
pub const OPIVV: u32 = 0b000;
/// FP vector-vector.
pub const OPFVV: u32 = 0b001;
/// Mask/reduction vector-vector.
pub const OPMVV: u32 = 0b010;
/// Integer vector-immediate.
pub const OPIVI: u32 = 0b011;
/// Integer vector-scalar.
pub const OPIVX: u32 = 0b100;
/// FP vector-scalar.
pub const OPFVF: u32 = 0b101;
/// Mask/move vector-scalar.
pub const OPMVX: u32 = 0b110;
/// Configuration (vsetvl family).
pub const OPCFG: u32 = 0b111;
