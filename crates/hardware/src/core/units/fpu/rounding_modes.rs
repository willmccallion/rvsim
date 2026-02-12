//! Floating-point rounding mode support.
//!
//! RISC-V defines five rounding modes (spec §11.2):
//!
//! | Value | Mode | Description                          |
//! |-------|------|--------------------------------------|
//! | 0b000 | RNE  | Round to Nearest, ties to Even       |
//! | 0b001 | RTZ  | Round towards Zero                   |
//! | 0b010 | RDN  | Round Down (towards −∞)              |
//! | 0b011 | RUP  | Round Up (towards +∞)                |
//! | 0b100 | RMM  | Round to Nearest, ties to Max Magnitude |
//!
//! This module will provide the `execute_with_rm` variant of the FPU that
//! accepts an explicit rounding mode, overriding the `fcsr.frm` default.
//!
//! **Status:** Stub — implementation pending as part of Phase 1.2 rounding
//! mode verification.

/// RISC-V rounding mode encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RoundingMode {
    /// Round to Nearest, ties to Even (default IEEE mode).
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
    pub fn from_bits(bits: u8) -> Option<Self> {
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
