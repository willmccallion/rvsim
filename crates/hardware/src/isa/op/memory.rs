//! Memory access widths and atomic memory operations.

/// Atomic memory operation types (RISC-V A extension).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AtomicOp {
    /// No atomic operation.
    #[default]
    None,

    /// Load-reserved (atomic load with reservation).
    Lr,

    /// Store-conditional (atomic store if reservation valid).
    Sc,

    /// Atomic swap.
    Swap,

    /// Atomic add.
    Add,

    /// Atomic XOR.
    Xor,

    /// Atomic AND.
    And,

    /// Atomic OR.
    Or,

    /// Atomic minimum (signed).
    Min,

    /// Atomic maximum (signed).
    Max,

    /// Atomic minimum (unsigned).
    Minu,

    /// Atomic maximum (unsigned).
    Maxu,
}

/// Memory access width for load and store operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemWidth {
    /// No memory operation.
    #[default]
    Nop,

    /// 8-bit byte access.
    Byte,

    /// 16-bit half-word access.
    Half,

    /// 32-bit word access.
    Word,

    /// 64-bit double-word access.
    Double,
}

impl MemWidth {
    /// Bytes moved by an access of this width; zero for `Nop`.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Nop => 0,
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
            Self::Double => 8,
        }
    }
}
