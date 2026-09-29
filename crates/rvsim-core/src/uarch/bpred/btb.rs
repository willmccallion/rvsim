//! Branch Target Buffer (BTB).
//!
//! The BTB is a set-associative cache that stores target addresses for control
//! flow instructions. It allows the fetch stage to predict the target of a
//! branch or jump before the instruction is decoded.

/// The kind of control instruction a BTB entry records: what fetch learns
/// about the instruction at a PC before it has been decoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BranchKind {
    /// A conditional branch.
    #[default]
    Conditional,
    /// A direct jump (`jal`); `call` when it links a return address.
    Jump {
        /// Pushes a return address.
        call: bool,
    },
    /// An indirect jump (`jalr`).
    Indirect {
        /// Pops a return address.
        returns: bool,
        /// Pushes a return address.
        call: bool,
    },
}

/// What a BTB hit tells fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtbHit {
    /// The last target the instruction went to.
    pub target: u64,
    /// The kind of control instruction.
    pub kind: BranchKind,
}

/// An entry in the Branch Target Buffer.
#[derive(Clone, Copy, Debug, Default)]
struct BtbEntry {
    /// The tag used to verify if this entry corresponds to the requested PC.
    tag: u64,
    /// The predicted target address.
    target: u64,
    /// The kind of control instruction at the tagged PC.
    kind: BranchKind,
    /// Indicates if this entry contains valid data.
    valid: bool,
}

/// Set-associative Branch Target Buffer.
#[derive(Debug)]
pub struct Btb {
    /// Flat array of entries: `num_sets * ways` elements.
    table: Vec<BtbEntry>,
    /// Number of sets (must be a power of 2).
    num_sets: usize,
    /// Number of ways (associativity).
    ways: usize,
    /// Per-set replacement pointer (round-robin index into the way).
    replace_ptr: Vec<u8>,
}

impl Btb {
    /// Creates a new set-associative Branch Target Buffer.
    ///
    /// # Arguments
    ///
    /// * `size` - Total number of entries. Must be a power of 2.
    /// * `ways` - Associativity (entries per set). Must be >= 1 and divide `size`.
    pub fn new(size: usize, ways: usize) -> Self {
        let ways = ways.max(1);
        let num_sets = (size / ways).max(1);
        debug_assert!(num_sets.is_power_of_two(), "BTB num_sets must be a power of 2");
        Self {
            table: vec![BtbEntry::default(); num_sets * ways],
            num_sets,
            ways,
            replace_ptr: vec![0; num_sets],
        }
    }

    /// Calculates the set index for a given program counter.
    #[inline]
    const fn set_index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) & (self.num_sets - 1)
    }

    /// The entry for the control instruction at `pc`, if the BTB holds one.
    pub fn lookup(&self, pc: u64) -> Option<BtbHit> {
        let set = self.set_index(pc);
        let base = set * self.ways;
        self.table[base..base + self.ways]
            .iter()
            .find(|e| e.valid && e.tag == pc)
            .map(|e| BtbHit { target: e.target, kind: e.kind })
    }

    /// Drops the entry for `pc`: the instruction there is not the control
    /// instruction the BTB recorded.
    pub fn invalidate(&mut self, pc: u64) {
        let set = self.set_index(pc);
        let base = set * self.ways;
        for e in &mut self.table[base..base + self.ways] {
            if e.valid && e.tag == pc {
                e.valid = false;
            }
        }
    }

    /// Records the control instruction at `pc`: its kind and latest target.
    ///
    /// If the tag already exists in the set, updates it in place. Otherwise,
    /// replaces the first invalid entry or uses round-robin replacement.
    pub fn update(&mut self, pc: u64, target: u64, kind: BranchKind) {
        let set = self.set_index(pc);
        let base = set * self.ways;
        let entry = BtbEntry { tag: pc, target, kind, valid: true };

        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if e.valid && e.tag == pc {
                *e = entry;
                return;
            }
        }

        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if !e.valid {
                *e = entry;
                return;
            }
        }

        let victim = self.replace_ptr[set] as usize % self.ways;
        self.replace_ptr[set] = ((victim + 1) % self.ways) as u8;
        self.table[base + victim] = entry;
    }
}
