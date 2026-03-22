//! Circular Shift Register (CSR) for O(1) incremental folded XOR of a GHR window.
//!
//! This is the standard technique from Seznec's TAGE papers: instead of recomputing
//! the XOR-fold of the full history on every access, each CSR is updated incrementally
//! when a new branch outcome is pushed into the GHR.

use crate::core::units::bru::Ghr;

/// A single Circular Shift Register for incremental folded XOR computation.
///
/// Maintains a `fold_width`-bit value that represents the XOR-fold of the
/// most recent `hist_length` bits of the GHR, updated incrementally in O(1)
/// per branch outcome push.
#[derive(Clone, Copy, Debug)]
pub struct FoldedHistory {
    /// Current folded value (only the low `fold_width` bits are meaningful).
    pub val: u64,
    /// Number of bits to fold into (e.g., `table_bits` for index, `tag_width` for tag).
    fold_width: usize,
    /// History length for this bank (how many GHR bits contribute).
    hist_length: usize,
}

impl FoldedHistory {
    /// Creates a new CSR that folds `hist_length` GHR bits into `fold_width` bits.
    pub const fn new(fold_width: usize, hist_length: usize) -> Self {
        Self { val: 0, fold_width, hist_length }
    }

    /// Incrementally updates the CSR when a new bit is pushed into the GHR.
    ///
    /// Must be called BEFORE `ghr.push()`. `old_bit` is `ghr.bit(hist_length - 1)`,
    /// the bit about to leave this bank's history window.
    ///
    /// Formula: `CSR = rotate_left(CSR, 1, fold_width) ^ new_bit ^ (old_bit << (hist_length % fold_width))`
    #[inline]
    pub const fn update(&mut self, new_bit: bool, old_bit: bool) {
        let w = self.fold_width;
        if w == 0 {
            return;
        }
        let mask = (1u64 << w) - 1;
        // Circular left rotate within fold_width bits.
        let msb = (self.val >> (w - 1)) & 1;
        self.val = ((self.val << 1) | msb) & mask;
        // XOR in the new bit at position 0.
        self.val ^= new_bit as u64;
        // XOR out the old (leaving) bit at its folded position.
        self.val ^= (old_bit as u64) << (self.hist_length % w);
        self.val &= mask;
    }

    /// Recomputes the CSR from scratch by replaying history bits oldest→newest.
    ///
    /// Used after `repair_history()` (misprediction recovery) and in
    /// `update_branch()` to reconstruct CSRs from a snapshot GHR.
    pub fn recompute(&mut self, ghr: &Ghr) {
        let w = self.fold_width;
        if w == 0 {
            self.val = 0;
            return;
        }
        let mask = (1u64 << w) - 1;
        self.val = 0;
        // Replay from oldest (position hist_length-1) to newest (position 0).
        // During replay, no bits leave the window (it grows from 0 to hist_length),
        // so old_bit is always false.
        for i in (0..self.hist_length).rev() {
            let msb = (self.val >> (w - 1)) & 1;
            self.val = ((self.val << 1) | msb) & mask;
            self.val ^= ghr.bit(i) as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folded_history_update_matches_recompute() {
        let hist_length = 44;
        let fold_width = 11;
        let mut ghr = Ghr::with_len(hist_length);
        let mut csr = FoldedHistory::new(fold_width, hist_length);

        for i in 0..200 {
            let bit = (i * 7 + 3) % 2 == 0;
            let old_bit = if hist_length > 0 { ghr.bit(hist_length - 1) } else { false };
            csr.update(bit, old_bit);
            ghr.push(bit);
        }

        let mut recomputed = FoldedHistory::new(fold_width, hist_length);
        recomputed.recompute(&ghr);
        assert_eq!(
            csr.val, recomputed.val,
            "Incremental CSR ({:#x}) != recomputed ({:#x})",
            csr.val, recomputed.val
        );
    }

    #[test]
    fn test_folded_history_various_widths() {
        for &(hist_len, fold_w) in &[(5, 3), (15, 9), (44, 11), (130, 10), (712, 11)] {
            let mut ghr = Ghr::with_len(hist_len);
            let mut csr = FoldedHistory::new(fold_w, hist_len);

            for i in 0..300 {
                let bit = i % 3 != 0;
                let old_bit = if hist_len > 0 { ghr.bit(hist_len - 1) } else { false };
                csr.update(bit, old_bit);
                ghr.push(bit);
            }

            let mut recomputed = FoldedHistory::new(fold_w, hist_len);
            recomputed.recompute(&ghr);
            assert_eq!(
                csr.val, recomputed.val,
                "Mismatch for hist_len={hist_len}, fold_w={fold_w}: {:#x} != {:#x}",
                csr.val, recomputed.val
            );
        }
    }
}
