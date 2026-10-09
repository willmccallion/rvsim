//! The `misa` register: which extensions a hart implements, and the ISA
//! strings that name them.

use crate::isa::csr::{
    MISA_DEFAULT_RV64GCB, MISA_EXT_A, MISA_EXT_B, MISA_EXT_C, MISA_EXT_D, MISA_EXT_F, MISA_EXT_I,
    MISA_EXT_M, MISA_EXT_S, MISA_EXT_U, MISA_EXT_V, MISA_XLEN_64,
};

/// A `misa` value parsed from an ISA string such as `"RV64IMAFDC"` or
/// `"rv64gcv"`.
///
/// The string names XLEN 64 and single-letter extensions (`G` is `IMAFD`),
/// case-insensitively. S and U are always set because the hart implements
/// both modes, and B because it implements Zba, Zbb and Zbs whatever the
/// string names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Misa(u64);

impl Misa {
    /// RV64IMAFDC with B, S and U, plus V when `with_v`.
    #[must_use]
    pub const fn rv64gcb(with_v: bool) -> Self {
        if with_v { Self(MISA_DEFAULT_RV64GCB | MISA_EXT_V) } else { Self(MISA_DEFAULT_RV64GCB) }
    }

    /// The register value.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// True when V is set.
    #[must_use]
    pub const fn has_v(self) -> bool {
        self.0 & MISA_EXT_V != 0
    }

    /// True when the hart has supervisor mode.
    #[must_use]
    pub const fn has_s(self) -> bool {
        self.0 & MISA_EXT_S != 0
    }

    /// The lowercase ISA string naming these extensions, e.g. `rv64imafdcv`.
    #[must_use]
    pub fn isa_string(self) -> String {
        let mut isa = String::from("rv64");
        for (letter, bit) in ISA_LETTERS {
            if self.0 & bit != 0 {
                isa.push(letter);
            }
        }
        isa
    }
}

impl std::str::FromStr for Misa {
    type Err = IsaStringError;

    fn from_str(isa: &str) -> Result<Self, Self::Err> {
        let upper = isa.to_ascii_uppercase();
        let Some(extensions) = upper.strip_prefix("RV64") else {
            return Err(IsaStringError::NotRv64(isa.to_string()));
        };
        let mut bits = MISA_XLEN_64 | MISA_EXT_B | MISA_EXT_S | MISA_EXT_U;
        for ext in extensions.chars() {
            bits |= match ext {
                'G' => MISA_EXT_I | MISA_EXT_M | MISA_EXT_A | MISA_EXT_F | MISA_EXT_D,
                'I' => MISA_EXT_I,
                'M' => MISA_EXT_M,
                'A' => MISA_EXT_A,
                'F' => MISA_EXT_F,
                'D' => MISA_EXT_D,
                'C' => MISA_EXT_C,
                'B' => MISA_EXT_B,
                'V' => MISA_EXT_V,
                other => return Err(IsaStringError::Unsupported(isa.to_string(), other)),
            };
        }
        if bits & MISA_EXT_I == 0 {
            return Err(IsaStringError::NoBase(isa.to_string()));
        }
        if bits & MISA_EXT_D != 0 && bits & MISA_EXT_F == 0 {
            return Err(IsaStringError::DWithoutF(isa.to_string()));
        }
        Ok(Self(bits))
    }
}

/// Why an ISA string does not describe a hart this simulator implements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsaStringError {
    /// The string does not start with `RV64`.
    NotRv64(String),
    /// The string names no `I` base.
    NoBase(String),
    /// The string names an extension the hart does not implement.
    Unsupported(String, char),
    /// The string names D without F, which D requires.
    DWithoutF(String),
}

impl std::fmt::Display for IsaStringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRv64(isa) => write!(f, "ISA string {isa:?} does not start with RV64"),
            Self::NoBase(isa) => write!(f, "ISA string {isa:?} has no I base"),
            Self::Unsupported(isa, ext) => {
                write!(f, "ISA string {isa:?} names unsupported extension {ext:?}")
            }
            Self::DWithoutF(isa) => write!(f, "ISA string {isa:?} has D without F"),
        }
    }
}

impl std::error::Error for IsaStringError {}

/// The single-letter extensions an ISA string can name, in canonical order.
const ISA_LETTERS: [(char, u64); 8] = [
    ('i', MISA_EXT_I),
    ('m', MISA_EXT_M),
    ('a', MISA_EXT_A),
    ('f', MISA_EXT_F),
    ('d', MISA_EXT_D),
    ('c', MISA_EXT_C),
    ('b', MISA_EXT_B),
    ('v', MISA_EXT_V),
];
