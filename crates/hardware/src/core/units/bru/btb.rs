//! Branch Target Buffer (BTB).
//!
//! The BTB is a set-associative cache that stores target addresses for control
//! flow instructions. It allows the fetch stage to predict the target of a
//! branch or jump before the instruction is decoded.

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

/// Set-associative Branch Target Buffer.
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
        debug_assert!(
            num_sets.is_power_of_two(),
            "BTB num_sets must be a power of 2"
        );
        Self {
            table: vec![BtbEntry::default(); num_sets * ways],
            num_sets,
            ways,
            replace_ptr: vec![0; num_sets],
        }
    }

    /// Calculates the set index for a given program counter.
    #[inline]
    fn set_index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) & (self.num_sets - 1)
    }

    /// Looks up a target address for the given program counter.
    ///
    /// Searches all ways in the indexed set for a tag match.
    ///
    /// # Returns
    ///
    /// The predicted target address if a valid entry exists and the tag matches,
    /// otherwise `None`.
    pub fn lookup(&self, pc: u64) -> Option<u64> {
        let set = self.set_index(pc);
        let base = set * self.ways;
        for w in 0..self.ways {
            let e = &self.table[base + w];
            if e.valid && e.tag == pc {
                return Some(e.target);
            }
        }
        None
    }

    /// Updates the BTB with a new target address for a specific program counter.
    ///
    /// If the tag already exists in the set, updates it in place. Otherwise,
    /// replaces the first invalid entry or uses round-robin replacement.
    pub fn update(&mut self, pc: u64, target: u64) {
        let set = self.set_index(pc);
        let base = set * self.ways;

        // Check for existing entry with same tag.
        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if e.valid && e.tag == pc {
                e.target = target;
                return;
            }
        }

        // Try to find an invalid (empty) slot.
        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if !e.valid {
                *e = BtbEntry {
                    tag: pc,
                    target,
                    valid: true,
                };
                return;
            }
        }

        // All ways valid — round-robin replacement.
        let victim = self.replace_ptr[set] as usize % self.ways;
        self.replace_ptr[set] = ((victim + 1) % self.ways) as u8;
        self.table[base + victim] = BtbEntry {
            tag: pc,
            target,
            valid: true,
        };
    }
}
