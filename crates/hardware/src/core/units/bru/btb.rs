//! Branch Target Buffer (BTB).
//!
//! The BTB is a direct-mapped cache that stores target addresses for control flow
//! instructions. It allows the fetch stage to predict the target of a branch or
//! jump before the instruction is decoded.

/// An entry in the Branch Target Buffer.
#[derive(Clone, Copy, Default)]
struct BtbEntry {
    /// The tag used to verify if this entry corresponds to the requested PC.
    tag: u64,
    /// The predicted target address.
    target: u64,
    /// Indicates if this entry contains valid data.
    valid: bool,
}

/// Branch Target Buffer structure.
pub struct Btb {
    /// The table of BTB entries.
    table: Vec<BtbEntry>,
    /// The total number of entries in the BTB.
    size: usize,
}

impl Btb {
    /// Creates a new Branch Target Buffer with the specified size.
    ///
    /// # Arguments
    ///
    /// * `size` - The number of entries in the BTB. Must be a power of 2.
    pub fn new(size: usize) -> Self {
        Self {
            table: vec![BtbEntry::default(); size],
            size,
        }
    }

    /// Calculates the index into the BTB table for a given program counter.
    ///
    /// Shifts the PC right by 2 bits (ignoring instruction alignment) and masks
    /// it against the table size.
    fn index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) & (self.size - 1)
    }

    /// Looks up a target address for the given program counter.
    ///
    /// # Arguments
    ///
    /// * `pc` - The program counter to look up.
    ///
    /// # Returns
    ///
    /// The predicted target address if a valid entry exists and the tag matches,
    /// otherwise `None`.
    pub fn lookup(&self, pc: u64) -> Option<u64> {
        let idx = self.index(pc);
        let e = self.table[idx];
        if e.valid && e.tag == pc {
            Some(e.target)
        } else {
            None
        }
    }

    /// Updates the BTB with a new target address for a specific program counter.
    ///
    /// Writes a new entry or overwrites an existing one at the calculated index.
    ///
    /// # Arguments
    ///
    /// * `pc` - The program counter of the branch or jump.
    /// * `target` - The resolved target address.
    pub fn update(&mut self, pc: u64, target: u64) {
        let idx = self.index(pc);
        self.table[idx] = BtbEntry {
            tag: pc,
            target,
            valid: true,
        };
    }
}
