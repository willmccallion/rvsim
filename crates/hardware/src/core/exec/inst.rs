//! A decoded instruction with the values of its source registers.

use crate::common::{InstSize, RegIdx};
use crate::core::exec::signals::ControlSignals;

/// A decoded instruction and the values of its source registers: what
/// executing it needs, whichever engine executes it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inst {
    /// Its address.
    pub pc: u64,
    /// Its encoding, expanded to 32 bits when it is compressed.
    pub bits: u32,
    /// Its encoded size.
    pub size: InstSize,
    /// Destination register.
    pub rd: RegIdx,
    /// First source register.
    pub rs1: RegIdx,
    /// Second source register.
    pub rs2: RegIdx,
    /// Third source register (fused multiply-add).
    pub rs3: RegIdx,
    /// Decoded immediate.
    pub imm: i64,
    /// Value of `rs1`.
    pub rv1: u64,
    /// Value of `rs2`.
    pub rv2: u64,
    /// Value of `rs3`.
    pub rv3: u64,
    /// What it does.
    pub ctrl: ControlSignals,
}

impl Inst {
    /// The instruction after this one in program order.
    #[must_use]
    pub const fn next_pc(&self) -> u64 {
        self.pc.wrapping_add(self.size.as_u64())
    }
}
