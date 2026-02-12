//! Memory ordering and fence operations.
//!
//! This module implements RISC-V memory ordering semantics for
//! FENCE instructions, including the predecessor/successor ordering
//! sets (I/O/R/W) and TSO fence variants.

/// Predecessor/Successor ordering bits for FENCE instructions.
///
/// RISC-V FENCE encoding (spec §2.7): the immediate field encodes
/// two 4-bit fields — predecessor (bits 27:24) and successor (bits 23:20) —
/// each with flags for I (instruction fetch), O (device output), R (read), W (write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceSet {
    /// Device input ordering.
    pub i: bool,
    /// Device output ordering.
    pub o: bool,
    /// Memory read ordering.
    pub r: bool,
    /// Memory write ordering.
    pub w: bool,
}

impl FenceSet {
    /// Decodes a 4-bit FENCE ordering set from an instruction field.
    pub fn from_bits(bits: u8) -> Self {
        Self {
            i: bits & 0b1000 != 0,
            o: bits & 0b0100 != 0,
            r: bits & 0b0010 != 0,
            w: bits & 0b0001 != 0,
        }
    }

    /// Encodes back to a 4-bit field.
    pub fn to_bits(self) -> u8 {
        ((self.i as u8) << 3) | ((self.o as u8) << 2) | ((self.r as u8) << 1) | (self.w as u8)
    }

    /// Returns true if no ordering bits are set (the fence is a no-op).
    pub fn is_empty(self) -> bool {
        !self.i && !self.o && !self.r && !self.w
    }

    /// Returns true if all ordering bits are set (full barrier).
    pub fn is_full(self) -> bool {
        self.i && self.o && self.r && self.w
    }
}

/// Decoded FENCE instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fence {
    /// Predecessor ordering set — operations before the fence.
    pub pred: FenceSet,
    /// Successor ordering set — operations after the fence.
    pub succ: FenceSet,
}

impl Fence {
    /// Decodes a FENCE instruction from the raw 32-bit encoding.
    ///
    /// # Arguments
    ///
    /// * `inst` - The raw 32-bit instruction encoding.
    ///
    /// # Returns
    ///
    /// A decoded `Fence` with predecessor and successor ordering sets.
    pub fn decode(inst: u32) -> Self {
        let pred_bits = ((inst >> 24) & 0xF) as u8;
        let succ_bits = ((inst >> 20) & 0xF) as u8;
        Self {
            pred: FenceSet::from_bits(pred_bits),
            succ: FenceSet::from_bits(succ_bits),
        }
    }

    /// Returns true if this is a FENCE.TSO instruction.
    ///
    /// FENCE.TSO is encoded as `FENCE rw, rw` with the TSO hint bit
    /// (bit 28) set in the fm field. In practice, we recognize FENCE.TSO
    /// as pred={R,W} and succ={R,W} (the minimal TSO barrier).
    pub fn is_tso(&self) -> bool {
        self.pred.r
            && self.pred.w
            && !self.pred.i
            && !self.pred.o
            && self.succ.r
            && self.succ.w
            && !self.succ.i
            && !self.succ.o
    }

    /// Returns true if both predecessor and successor sets have no bits set.
    pub fn is_nop(&self) -> bool {
        self.pred.is_empty() && self.succ.is_empty()
    }

    /// Returns true if this is a full IORW,IORW barrier.
    pub fn is_full_barrier(&self) -> bool {
        self.pred.is_full() && self.succ.is_full()
    }
}
