//! The operand formats an operation computes in.

use crate::isa::fp::FpFormat;

/// The width an integer operation computes at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntWidth {
    /// 32 bits, the result sign-extended: the W-suffixed operations.
    Word,
    /// 64 bits.
    Double,
}

/// What an operation's operands are: integers of a width, or floats of a
/// format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandFormat {
    /// Integers.
    Int(IntWidth),
    /// Floating-point values.
    Fp(FpFormat),
}

impl Default for OperandFormat {
    fn default() -> Self {
        Self::Int(IntWidth::Double)
    }
}

impl OperandFormat {
    /// The width an integer unit computes at: 64 bits for anything but a
    /// W-operation, an FP operation's address add included.
    #[must_use]
    pub const fn int_width(self) -> IntWidth {
        match self {
            Self::Int(width) => width,
            Self::Fp(_) => IntWidth::Double,
        }
    }

    /// The format an FP unit computes in. Decode gives every FP operation
    /// an FP format, so the double fallback is never used by one.
    #[must_use]
    pub const fn fp_format(self) -> FpFormat {
        match self {
            Self::Fp(format) => format,
            Self::Int(_) => FpFormat::Double,
        }
    }
}
