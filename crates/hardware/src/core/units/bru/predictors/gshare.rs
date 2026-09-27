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

use crate::core::units::bru::ras::RasSnapshot;
use crate::core::units::bru::{BranchPredictor, Ghr, btb::Btb, ras::Ras};

/// Size of the Pattern History Table (2^12 entries).
const TABLE_BITS: usize = 12;
/// Total number of entries in the PHT.
const TABLE_SIZE: usize = 1 << TABLE_BITS;

/// `GShare` Predictor structure.
#[derive(Debug)]
pub struct GSharePredictor {
    /// Global history as fetched, which predictions use.
    ghr: u64,
    /// Global history through the last committed branch.
    commit_ghr: u64,
    /// Pattern History Table containing 2-bit saturating counters.
    pht: Vec<u8>,
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,
}

impl GSharePredictor {
    /// Creates a new `GShare` Predictor.
    pub fn new(btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        Self {
            ghr: 0,
            commit_ghr: 0,
            pht: vec![1; TABLE_SIZE],
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
        }
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

impl BranchPredictor for GSharePredictor {
    /// Predicts branch direction and target.
    ///
    /// Returns true if the 2-bit counter at the hashed index is 2 or 3 (Taken).
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        let idx = Self::index(pc, self.ghr);
        let counter = self.pht[idx];
        let taken = counter >= 2;

        if taken { (true, self.btb.lookup(pc)) } else { (false, None) }
    }

    /// Updates the predictor with the actual branch outcome.
    ///
    /// Trains the 2-bit counter the prediction read, found from the history
    /// the branch was predicted with.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr) {
        let idx = Self::index(pc, ghr_snapshot.val());
        let counter = self.pht[idx];

        if taken && counter < 3 {
            self.pht[idx] += 1;
        } else if !taken && counter > 0 {
            self.pht[idx] -= 1;
        }

        self.commit_ghr = Self::shifted(ghr_snapshot.val(), taken);

        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    /// Predicts the target of a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    fn push_return(&mut self, ret_addr: u64) {
        self.ras.push(ret_addr);
    }

    fn pop_return(&mut self) -> Option<u64> {
        self.ras.pop()
    }

    fn speculate(&mut self, _pc: u64, taken: bool) {
        self.ghr = Self::shifted(self.ghr, taken);
    }

    fn snapshot_history(&self) -> Ghr {
        Ghr::new(self.ghr)
    }

    fn repair_history(&mut self, ghr: &Ghr) {
        self.ghr = ghr.val();
    }

    fn snapshot_ras(&self) -> RasSnapshot {
        self.ras.snapshot()
    }

    fn restore_ras(&mut self, snapshot: RasSnapshot) {
        self.ras.restore(snapshot);
    }

    fn update_btb(&mut self, pc: u64, target: u64) {
        self.btb.update(pc, target);
    }

    fn repair_to_committed(&mut self) {
        self.ghr = self.commit_ghr;
    }
}
