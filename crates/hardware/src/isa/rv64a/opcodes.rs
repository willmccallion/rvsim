//! RISC-V Atomic Extension (A) Opcodes.

/// Atomic Memory Operation opcode (0b0101111).
/// Used for all AMO instructions (LR, SC, AMOADD, etc.).
pub const OP_AMO: u32 = 0b0101111;
