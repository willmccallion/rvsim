//! Floating-point rounding modes and accrued exception flags (`fcsr`).

use std::ops::BitOr;

/// RISC-V rounding mode encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RoundingMode {
    /// Round to Nearest, ties to Even (default IEEE mode).
    #[default]
    Rne = 0b000,
    /// Round towards Zero.
    Rtz = 0b001,
    /// Round Down (towards −∞).
    Rdn = 0b010,
    /// Round Up (towards +∞).
    Rup = 0b011,
    /// Round to Nearest, ties to Max Magnitude.
    Rmm = 0b100,
}

impl RoundingMode {
    /// Decodes a 3-bit rounding mode field from an instruction or `fcsr.frm`.
    ///
    /// Returns `None` for reserved encodings (0b101, 0b110) and the dynamic
    /// sentinel (0b111), which must be resolved to `fcsr.frm` by the caller.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits & 0x7 {
            0b000 => Some(Self::Rne),
            0b001 => Some(Self::Rtz),
            0b010 => Some(Self::Rdn),
            0b011 => Some(Self::Rup),
            0b100 => Some(Self::Rmm),
            _ => None, // 0b101, 0b110 reserved; 0b111 = dynamic
        }
    }
}

/// An IEEE 754 binary format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpFormat {
    /// binary16.
    Half,
    /// binary32.
    Single,
    /// binary64.
    Double,
}

impl FpFormat {
    /// The widths of its exponent and fraction fields, in bits.
    const fn field_bits(self) -> (u32, u32) {
        match self {
            Self::Half => (5, 10),
            Self::Single => (8, 23),
            Self::Double => (11, 52),
        }
    }
}

/// The class `FCLASS` reports a floating-point value in, numbered by the
/// bit of its one-hot result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpClass {
    /// −∞.
    NegInfinity = 0,
    /// A negative normal number.
    NegNormal = 1,
    /// A negative subnormal number.
    NegSubnormal = 2,
    /// −0.
    NegZero = 3,
    /// +0.
    PosZero = 4,
    /// A positive subnormal number.
    PosSubnormal = 5,
    /// A positive normal number.
    PosNormal = 6,
    /// +∞.
    PosInfinity = 7,
    /// A signaling NaN.
    SignalingNan = 8,
    /// A quiet NaN.
    QuietNan = 9,
}

impl FpClass {
    /// The class of the value whose `format` encoding is the low bits of
    /// `bits`.
    #[must_use]
    pub const fn of(format: FpFormat, bits: u64) -> Self {
        let (exp_bits, frac_bits) = format.field_bits();
        let frac = bits & ((1 << frac_bits) - 1);
        let exp = (bits >> frac_bits) & ((1 << exp_bits) - 1);
        let is_negative = (bits >> (frac_bits + exp_bits)) & 1 != 0;
        let exp_all_ones = (1 << exp_bits) - 1;
        let is_quiet = (frac >> (frac_bits - 1)) & 1 != 0;
        match (exp, frac, is_negative) {
            (e, f, _) if e == exp_all_ones && f != 0 => {
                if is_quiet {
                    Self::QuietNan
                } else {
                    Self::SignalingNan
                }
            }
            (e, _, true) if e == exp_all_ones => Self::NegInfinity,
            (e, _, false) if e == exp_all_ones => Self::PosInfinity,
            (0, 0, true) => Self::NegZero,
            (0, 0, false) => Self::PosZero,
            (0, _, true) => Self::NegSubnormal,
            (0, _, false) => Self::PosSubnormal,
            (_, _, true) => Self::NegNormal,
            (_, _, false) => Self::PosNormal,
        }
    }

    /// `FCLASS`'s one-hot result for the class.
    #[must_use]
    pub const fn mask(self) -> u64 {
        1 << self as u64
    }
}

/// The `frm` CSR: a rounding mode, or a reserved encoding (5 to 7) under
/// which every dynamically rounded instruction is illegal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "u8", into = "u8")]
pub struct Frm(u8);

impl From<u8> for Frm {
    fn from(bits: u8) -> Self {
        Self::from_bits(u64::from(bits))
    }
}

impl From<Frm> for u8 {
    fn from(frm: Frm) -> Self {
        frm.0
    }
}

impl Frm {
    /// Takes the low three bits of `bits`, as a CSR write does.
    pub const fn from_bits(bits: u64) -> Self {
        Self((bits & 0x7) as u8)
    }

    /// The 3-bit encoding.
    pub const fn bits(self) -> u64 {
        self.0 as u64
    }

    /// The mode it selects, or `None` for a reserved encoding.
    pub const fn rounding_mode(self) -> Option<RoundingMode> {
        RoundingMode::from_bits(self.0)
    }
}

/// The rounding an instruction's rounding-mode field selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// The mode the field names.
    Static(RoundingMode),
    /// The mode `fcsr.frm` holds when the instruction executes.
    Dynamic,
}

impl Rounding {
    /// The field value that selects `fcsr.frm`.
    const DYNAMIC_FIELD: u32 = 0b111;

    /// The rounding a 3-bit rounding-mode field selects, or `None` for the
    /// reserved 0b101 and 0b110.
    #[must_use]
    pub const fn from_field(field: u32) -> Option<Self> {
        if field == Self::DYNAMIC_FIELD {
            return Some(Self::Dynamic);
        }
        match RoundingMode::from_bits(field as u8) {
            Some(mode) => Some(Self::Static(mode)),
            None => None,
        }
    }
}

/// Floating-point exception flags (RISC-V `fcsr.fflags`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "u8", into = "u8")]
pub struct FpFlags(u8);

impl From<u8> for FpFlags {
    fn from(bits: u8) -> Self {
        Self::from_bits(bits)
    }
}

impl From<FpFlags> for u8 {
    fn from(flags: FpFlags) -> Self {
        flags.bits()
    }
}

impl FpFlags {
    /// No exceptions raised.
    pub const NONE: Self = Self(0);
    /// Invalid Operation.
    pub const NV: Self = Self(1 << 4);
    /// Divide by Zero.
    pub const DZ: Self = Self(1 << 3);
    /// Overflow.
    pub const OF: Self = Self(1 << 2);
    /// Underflow.
    pub const UF: Self = Self(1 << 1);
    /// Inexact.
    pub const NX: Self = Self(1 << 0);

    /// Creates `FpFlags` from a raw 5-bit value.
    pub const fn from_bits(val: u8) -> Self {
        Self(val & 0x1F)
    }

    /// The flags a CSR write of `val` sets: its low five bits.
    #[must_use]
    pub const fn from_csr(val: u64) -> Self {
        Self::from_bits(val as u8)
    }

    /// Returns the raw 5-bit flag value for writing into `fcsr`.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns true if no flags are set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns true if the specified flag is set.
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Both sets of flags.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOr for FpFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
