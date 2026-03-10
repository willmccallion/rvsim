//! Instruction size newtype for RISC-V instruction encoding widths.
//!
//! RISC-V supports two instruction sizes:
//! - 16-bit compressed (RVC) instructions
//! - 32-bit standard instructions

/// The size of an instruction in bytes (2 for compressed RVC, 4 for standard RV64).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstSize {
    /// 16-bit compressed instruction (RVC).
    Compressed,
    /// 32-bit standard instruction.
    #[default]
    Standard,
}

impl InstSize {
    /// Returns the instruction size in bytes as a `u64`.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        match self {
            Self::Compressed => 2,
            Self::Standard => 4,
        }
    }
}
