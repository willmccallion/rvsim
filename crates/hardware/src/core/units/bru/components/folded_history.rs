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

    /// Recomputes the CSR from scratch using word-level XOR-folding.
    ///
    /// Used after `repair_history()` (misprediction recovery) and in
    /// `update_branch()` to reconstruct CSRs from a snapshot GHR.
    ///
    /// Algorithm: for each 64-bit GHR word, XOR-fold it into `fold_width` bits,
    /// rotate by `(word_idx * 64) % fold_width` to correct alignment, then XOR
    /// into accumulator.
    pub fn recompute(&mut self, ghr: &Ghr) {
        let w = self.fold_width;
        if w == 0 {
            self.val = 0;
            return;
        }
        let mask = (1u64 << w) - 1;
        let mut result = 0u64;
        let num_words = self.hist_length.div_ceil(64);

        for word_idx in 0..num_words {
            let mut word = ghr.word(word_idx);
            // Mask last word to hist_length boundary.
            let bits_in_word = (self.hist_length - word_idx * 64).min(64);
            if bits_in_word < 64 {
                word &= (1u64 << bits_in_word) - 1;
            }
            if word == 0 {
                continue;
            }

            // Fold 64-bit word into w bits by XOR-ing w-bit chunks.
            let mut folded = 0u64;
            let mut v = word;
            while v != 0 {
                folded ^= v & mask;
                v >>= w;
            }

            // Rotate to correct alignment: bit position `word_idx*64 + b` in the
            // GHR maps to CSR position `(word_idx*64 + b) % w`. The fold above
            // placed bits as if `word_idx*64 == 0`, so rotate left by the offset.
            let rot = (word_idx * 64) % w;
            if rot > 0 {
                folded = ((folded << rot) | (folded >> (w - rot))) & mask;
            }
            result ^= folded;
        }
        self.val = result & mask;
    }

    /// Reference implementation: recomputes by replaying history bits one at a time.
    /// Used only in tests to verify the fast word-level `recompute()`.
    #[cfg(test)]
    pub fn recompute_reference(&mut self, ghr: &Ghr) {
        let w = self.fold_width;
        if w == 0 {
            self.val = 0;
            return;
        }
        let mask = (1u64 << w) - 1;
        self.val = 0;
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

    #[test]
    fn test_fast_recompute_matches_reference() {
        // All TAGE and ITTAGE history length / fold width combos.
        let cases = [
            // TAGE-like: table_bits=11, tag_widths 9-10, hist lengths up to 712
            (5, 11), (5, 10), (5, 9), (5, 8),
            (15, 11), (15, 10), (15, 9),
            (44, 11), (44, 10),
            (130, 11), (130, 10),
            (247, 11), (247, 10),
            (375, 11), (375, 10),
            (512, 11), (512, 10),
            (712, 11), (712, 10), (712, 9),
            // ITTAGE-like: shorter histories
            (4, 9), (8, 9), (16, 10), (32, 10),
            (64, 11), (128, 11), (256, 11), (512, 11),
            // Edge cases
            (1, 1), (2, 1), (63, 7), (64, 8), (65, 8), (127, 10), (128, 10),
        ];

        for &(hist_len, fold_w) in &cases {
            let mut ghr = Ghr::with_len(hist_len);
            for i in 0..500u64 {
                ghr.push((i.wrapping_mul(7) ^ i.wrapping_mul(13)) & 1 != 0);
            }

            let mut fast = FoldedHistory::new(fold_w, hist_len);
            fast.recompute(&ghr);

            let mut reference = FoldedHistory::new(fold_w, hist_len);
            reference.recompute_reference(&ghr);

            assert_eq!(
                fast.val, reference.val,
                "Fast vs reference mismatch for hist_len={hist_len}, fold_w={fold_w}: \
                 fast={:#x} ref={:#x}",
                fast.val, reference.val
            );
        }
    }
}
