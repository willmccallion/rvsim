//! `GShare` Branch Predictor.
//!
//! `GShare` correlates global branch history with the program counter using an XOR
//! hash. This allows the predictor to distinguish the same branch instruction
//! in different execution contexts.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `predict()`: O(1)
//!   - `update()`: O(1)
//! - **Space Complexity:** O(2^N) where N is the history length (12 bits = 4KB for 2-bit counters)
//! - **Hardware Cost:** Moderate - single PHT lookup, XOR, and counter update
//! - **Best Case:** Correlated branches where outcome depends on recent history
//! - **Worst Case:** Uncorrelated branches or history length too short/long for pattern

use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};

/// Size of the Pattern History Table (2^12 entries).
const TABLE_BITS: usize = 12;
/// Total number of entries in the PHT.
const TABLE_SIZE: usize = 1 << TABLE_BITS;

/// `GShare` Predictor structure.
#[derive(Debug)]
pub struct GSharePredictor {
    /// Global history as fetched, which predictions use.
    ghr: u64,
    /// Pattern History Table containing 2-bit saturating counters.
    pht: Vec<u8>,
}

/// The global history a `GShare` prediction was made with.
#[derive(Clone, Copy, Debug)]
pub struct GShareHistory {
    ghr: u64,
}

impl Default for GSharePredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl GSharePredictor {
    /// Creates a new `GShare` Predictor.
    pub fn new() -> Self {
        Self { ghr: 0, pht: vec![1; TABLE_SIZE] }
    }

    /// The speculative global history, newest outcome in bit 0.
    pub const fn history(&self) -> u64 {
        self.ghr
    }

    /// The Pattern History Table index for `pc` under global history `ghr`:
    /// the XOR of the two.
    const fn index(pc: u64, ghr: u64) -> usize {
        let pc_part = (pc >> 2) & ((TABLE_SIZE as u64) - 1);
        let ghr_part = ghr & ((TABLE_SIZE as u64) - 1);
        (pc_part ^ ghr_part) as usize
    }

    const fn shifted(ghr: u64, taken: bool) -> u64 {
        ((ghr << 1) | taken as u64) & ((TABLE_SIZE as u64) - 1)
    }
}

impl DirectionPredictor for GSharePredictor {
    type History = GShareHistory;

    /// Predicts taken when the 2-bit counter at the hashed index is 2 or 3.
    fn lookup(&self, pc: u64) -> (bool, GShareHistory) {
        let taken = self.pht[Self::index(pc, self.ghr)] >= 2;
        (taken, GShareHistory { ghr: self.ghr })
    }

    fn unconditional(&self, _pc: u64) -> GShareHistory {
        GShareHistory { ghr: self.ghr }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, _history: &GShareHistory) {
        self.ghr = Self::shifted(self.ghr, taken);
    }

    fn squash(&mut self, history: &GShareHistory) {
        self.ghr = history.ghr;
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &GShareHistory) {
        self.ghr = Self::shifted(history.ghr, taken);
    }

    /// Trains the 2-bit counter the prediction read.
    fn commit(&mut self, pc: u64, retired: Retired, history: &GShareHistory) {
        if retired.class != BranchClass::Conditional {
            return;
        }
        let counter = &mut self.pht[Self::index(pc, history.ghr)];
        if retired.taken && *counter < 3 {
            *counter += 1;
        } else if !retired.taken && *counter > 0 {
            *counter -= 1;
        }
    }
}
