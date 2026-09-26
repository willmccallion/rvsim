//! Branch Predictor Interface.
//!
//! This module defines the `BranchPredictor` trait that all branch prediction
//! implementations must adhere to. It provides a common interface for
//! predicting conditional branches, indirect jumps (via BTB), and function
//! returns (via RAS).

use super::ras::RasSnapshot;

/// Maximum number of u64 words in a GHR. 16 × 64 = 1024 bits.
/// This is a capacity bound — the effective history length comes from config.
const GHR_MAX_WORDS: usize = 16;

/// Global History Register snapshot.
///
/// A fixed-capacity shift register that stores branch outcome history.
/// The effective history length (`len`) is determined by the predictor
/// configuration (e.g., `max(hist_lengths)` for TAGE), while the storage
/// capacity is bounded at compile time at [`GHR_MAX_WORDS`] × 64 = 1024 bits.
///
/// Bit 0 is the most recently pushed outcome. Snapshots are captured at
/// fetch time and carried through pipeline latches so that update and
/// repair operations use the correct history state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ghr {
    /// Bit storage. `bits[0]` bit 0 = position 0 (most recent).
    /// `bits[0]` bit 63 = position 63, `bits[1]` bit 0 = position 64, etc.
    bits: [u64; GHR_MAX_WORDS],
    /// Effective history length in bits, from config. Bits beyond this are
    /// masked off on push and ignored by consumers.
    len: u16,
}

impl Default for Ghr {
    fn default() -> Self {
        Self { bits: [0; GHR_MAX_WORDS], len: 0 }
    }
}

impl Ghr {
    /// Creates a new GHR from a u64 value (64-bit history).
    ///
    /// Backward-compatible constructor for predictors that only use 64 bits
    /// of history (`GShare`, Tournament, Perceptron).
    #[inline]
    pub const fn new(val: u64) -> Self {
        let mut bits = [0u64; GHR_MAX_WORDS];
        bits[0] = val;
        Self { bits, len: 64 }
    }

    /// Creates a zero-initialized GHR with the given effective history length.
    ///
    /// # Panics
    ///
    /// Panics if `max_bits` exceeds the compile-time capacity (1024 bits).
    pub fn with_len(max_bits: usize) -> Self {
        assert!(
            max_bits <= GHR_MAX_WORDS * 64,
            "GHR: requested {max_bits} bits exceeds capacity of {} bits",
            GHR_MAX_WORDS * 64,
        );
        Self { bits: [0; GHR_MAX_WORDS], len: max_bits as u16 }
    }

    /// Returns the low 64 bits of the GHR.
    ///
    /// Backward-compatible accessor for predictors that only use 64 bits.
    #[inline]
    pub const fn val(&self) -> u64 {
        self.bits[0]
    }

    /// Returns the raw u64 word at the given index into the backing store.
    ///
    /// Returns 0 for out-of-bounds indices. Useful for word-level algorithms
    /// that process the GHR in 64-bit chunks.
    #[inline]
    pub const fn word(&self, idx: usize) -> u64 {
        if idx < GHR_MAX_WORDS { self.bits[idx] } else { 0 }
    }

    /// Returns the bit at position `pos` (0 = most recent outcome).
    ///
    /// Returns `false` for positions beyond storage capacity.
    #[inline]
    pub const fn bit(&self, pos: usize) -> bool {
        let word_idx = pos / 64;
        let bit_idx = pos % 64;
        if word_idx >= GHR_MAX_WORDS {
            return false;
        }
        (self.bits[word_idx] >> bit_idx) & 1 != 0
    }

    /// Pushes a new branch outcome into the register (left shift by 1).
    ///
    /// All positions shift up by 1 (position K moves to K+1). The new
    /// outcome is inserted at position 0. Bits beyond `len` are masked off.
    pub fn push(&mut self, taken: bool) {
        for i in (1..GHR_MAX_WORDS).rev() {
            self.bits[i] = (self.bits[i] << 1) | (self.bits[i - 1] >> 63);
        }
        self.bits[0] = (self.bits[0] << 1) | (taken as u64);

        let len = self.len as usize;
        if len > 0 && len < GHR_MAX_WORDS * 64 {
            let top_word_idx = len / 64;
            let top_bit_count = len % 64;
            if top_word_idx < GHR_MAX_WORDS {
                if top_bit_count > 0 {
                    self.bits[top_word_idx] &= (1u64 << top_bit_count) - 1;
                } else {
                    self.bits[top_word_idx] = 0;
                }
                for w in &mut self.bits[top_word_idx + 1..] {
                    *w = 0;
                }
            }
        }
    }
}

/// Trait for branch prediction algorithms.
///
/// Defines the interface that all branch prediction implementations
/// must provide for predicting branch directions, targets, and managing
/// return address prediction.
pub trait BranchPredictor {
    /// Predicts whether a branch instruction will be taken and its target address.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>);

    /// Updates the branch predictor with actual branch outcome.
    ///
    /// Called at commit time to train the predictor with the actual
    /// taken/not-taken decision and target address. The `ghr_snapshot`
    /// is the GHR captured at fetch time — predictors that use history
    /// for indexing must use this snapshot (not their live GHR) to
    /// ensure they train the same table entries that were consulted
    /// during prediction.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr);

    /// Predicts the target address for a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64>;

    /// Pushes the return address of a call the moment it is fetched.
    fn push_return(&mut self, ret_addr: u64);

    /// Pops the predicted target of a return the moment it is fetched.
    fn pop_return(&mut self) -> Option<u64>;

    /// Speculatively updates the GHR with a predicted branch outcome.
    ///
    /// Called at fetch time after `predict_branch` to keep the GHR
    /// up-to-date for subsequent predictions before resolution.
    fn speculate(&mut self, _pc: u64, _taken: bool) {}

    /// Returns a snapshot of the current GHR for later repair.
    ///
    /// Called at fetch time before `speculate` so the snapshot can be
    /// carried through the pipeline and used at resolution to restore
    /// the GHR to the correct state.
    fn snapshot_history(&self) -> Ghr {
        Ghr::default()
    }

    /// Restores the GHR to a previously captured snapshot.
    ///
    /// Called at resolution time (execute) before `update_branch` so
    /// the predictor trains on the correct history state.
    fn repair_history(&mut self, _ghr: &Ghr) {}

    /// Captures the RAS state fetched pushes and pops are undone to.
    fn snapshot_ras(&self) -> RasSnapshot {
        RasSnapshot::default()
    }

    /// Undoes the RAS pushes and pops since `snapshot` was taken.
    fn restore_ras(&mut self, _snapshot: RasSnapshot) {}

    /// Updates only the BTB with a jump target (no direction training).
    ///
    /// Called at execute time for unconditional jumps (JAL/JALR) so the BTB
    /// learns the target without polluting direction predictor state.
    fn update_btb(&mut self, _pc: u64, _target: u64) {}

    /// Resets speculative GHR to the committed GHR state.
    ///
    /// Called on full pipeline flushes (trap, MRET/SRET, FENCE.I) where the
    /// speculative history may contain wrong-path branch outcomes.
    fn repair_to_committed(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyPredictor;
    impl BranchPredictor for DummyPredictor {
        fn predict_branch(&self, _pc: u64) -> (bool, Option<u64>) {
            (false, None)
        }
        fn update_branch(&mut self, _pc: u64, _taken: bool, _target: Option<u64>, _ghr: &Ghr) {}
        fn predict_btb(&self, _pc: u64) -> Option<u64> {
            None
        }
        fn push_return(&mut self, _ret_addr: u64) {}
        fn pop_return(&mut self) -> Option<u64> {
            None
        }
    }

    #[test]
    fn test_branch_predictor_defaults() {
        let mut predictor = DummyPredictor;
        predictor.speculate(0x1000, true);
        assert_eq!(predictor.snapshot_history(), Ghr::default());
        predictor.repair_history(&Ghr::new(42));
        assert_eq!(predictor.snapshot_ras(), RasSnapshot::default());
        predictor.restore_ras(RasSnapshot::default());
    }

    #[test]
    fn test_ghr_new_and_val() {
        let ghr = Ghr::new(0xDEAD_BEEF);
        assert_eq!(ghr.val(), 0xDEAD_BEEF);
    }

    #[test]
    fn test_ghr_with_len() {
        let ghr = Ghr::with_len(712);
        assert_eq!(ghr.len, 712);
        assert_eq!(ghr.val(), 0);
    }

    #[test]
    fn test_ghr_push_and_bit() {
        let mut ghr = Ghr::with_len(128);
        ghr.push(true);
        ghr.push(false);
        ghr.push(true);
        assert!(ghr.bit(0)); // newest = true
        assert!(!ghr.bit(1)); // second = false
        assert!(ghr.bit(2)); // oldest = true
        assert!(!ghr.bit(3)); // never pushed = false
    }

    #[test]
    fn test_ghr_push_across_word_boundary() {
        let mut ghr = Ghr::with_len(128);
        for i in 0..65 {
            ghr.push(i == 0); // only the first push is true, now at position 64
        }
        assert!(ghr.bit(64));
        assert!(!ghr.bit(63));
        assert!(!ghr.bit(65));
    }

    #[test]
    fn test_ghr_push_masks_to_len() {
        let mut ghr = Ghr::with_len(5);
        for _ in 0..10 {
            ghr.push(true);
        }
        assert!(ghr.bit(0));
        assert!(ghr.bit(4));
        assert!(!ghr.bit(5)); // masked off
    }

    #[test]
    fn test_ghr_default() {
        let ghr = Ghr::default();
        assert_eq!(ghr.len, 0);
        assert_eq!(ghr.val(), 0);
        assert!(!ghr.bit(0));
    }

    #[test]
    fn test_ghr_copy_semantics() {
        let mut ghr = Ghr::with_len(64);
        ghr.push(true);
        let snapshot = ghr; // Copy
        ghr.push(false); // mutate original
        assert!(snapshot.bit(0)); // snapshot unchanged
        assert!(!ghr.bit(0)); // original has the new push
    }
}
