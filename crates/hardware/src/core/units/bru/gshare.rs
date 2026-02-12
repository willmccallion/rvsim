//! GShare Branch Predictor.
//!
//! GShare correlates global branch history with the program counter using an XOR
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

use super::{BranchPredictor, btb::Btb, ras::Ras};

/// Size of the Pattern History Table (2^12 entries).
const TABLE_BITS: usize = 12;
/// Total number of entries in the PHT.
const TABLE_SIZE: usize = 1 << TABLE_BITS;

/// GShare Predictor structure.
pub struct GSharePredictor {
    /// Global History Register storing recent branch outcomes.
    ghr: u64,
    /// Pattern History Table containing 2-bit saturating counters.
    pht: Vec<u8>,
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,
}

impl GSharePredictor {
    /// Creates a new GShare Predictor.
    pub fn new(btb_size: usize, ras_size: usize) -> Self {
        Self {
            ghr: 0,
            pht: vec![1; TABLE_SIZE],
            btb: Btb::new(btb_size),
            ras: Ras::new(ras_size),
        }
    }

    /// Calculates the index into the Pattern History Table.
    ///
    /// Computes the XOR of the PC (shifted) and the Global History Register.
    fn index(&self, pc: u64) -> usize {
        let pc_part = (pc >> 2) & ((TABLE_SIZE as u64) - 1);
        let ghr_part = self.ghr & ((TABLE_SIZE as u64) - 1);
        (pc_part ^ ghr_part) as usize
    }
}

impl BranchPredictor for GSharePredictor {
    /// Predicts branch direction and target.
    ///
    /// Returns true if the 2-bit counter at the hashed index is 2 or 3 (Taken).
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        let idx = self.index(pc);
        let counter = self.pht[idx];
        let taken = counter >= 2;

        if taken {
            (true, self.btb.lookup(pc))
        } else {
            (false, None)
        }
    }

    /// Updates the predictor with the actual branch outcome.
    ///
    /// Updates the 2-bit saturating counter in the PHT and shifts the new
    /// outcome into the Global History Register.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>) {
        let idx = self.index(pc);
        let counter = self.pht[idx];

        if taken && counter < 3 {
            self.pht[idx] += 1;
        } else if !taken && counter > 0 {
            self.pht[idx] -= 1;
        }

        self.ghr = ((self.ghr << 1) | if taken { 1 } else { 0 }) & ((TABLE_SIZE as u64) - 1);

        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    /// Predicts the target of a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    /// Handles a function call by pushing the return address to the RAS.
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras.push(ret_addr);
        self.btb.update(pc, target);
    }

    /// Predicts the return address using the RAS.
    fn predict_return(&self) -> Option<u64> {
        self.ras.top()
    }

    /// Handles a function return by popping from the RAS.
    fn on_return(&mut self) {
        self.ras.pop();
    }
}
