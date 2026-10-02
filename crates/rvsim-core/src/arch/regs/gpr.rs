//! RISC-V General-Purpose Register file (x0–x31, with x0 hardwired to zero).

use crate::isa::reg::RegIdx;

/// General-Purpose Register file.
///
/// Contains 32 general-purpose registers used for integer operations. Register `x0`
/// is hardwired to zero and cannot be modified.
#[derive(Debug)]
pub struct Gpr {
    regs: [u64; 32],
}

impl Default for Gpr {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpr {
    /// Creates a new general-purpose register file with all registers initialized to zero.
    pub const fn new() -> Self {
        Self { regs: [0; 32] }
    }

    /// Reads a general-purpose register value. Register `x0` always returns 0.
    pub const fn read(&self, idx: RegIdx) -> u64 {
        if idx.is_zero() { 0 } else { self.regs[idx.as_usize()] }
    }

    /// Writes a value to a general-purpose register. Writes to `x0` are ignored.
    pub const fn write(&mut self, idx: RegIdx, val: u64) {
        if !idx.is_zero() {
            self.regs[idx.as_usize()] = val;
        }
    }
}

/// Two registers per line, `x0` first, as hexadecimal.
impl std::fmt::Display for Gpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, [even, odd]) in self.regs.as_chunks::<2>().0.iter().enumerate() {
            let first = i * 2;
            writeln!(f, "x{first:<2}={even:#018x} x{:<2}={odd:#018x}", first + 1)?;
        }
        Ok(())
    }
}
