//! TAGE (Tagged Geometric History Length) Branch Predictor with Loop Predictor.
//!
//! TAGE uses a base bimodal predictor and multiple tagged banks indexed with
//! geometrically increasing history lengths. It provides high accuracy by
//! matching long history patterns while falling back to shorter histories
//! or the base predictor when necessary.
//!
//! The loop predictor detects counted loops and overrides TAGE when it has
//! high confidence in the loop's trip count.
//!
//! # Incremental Folded History (CSR)
//!
//! Index and tag computations use Circular Shift Register (CSR) folded history.
//! Instead of recomputing the XOR-fold of the full history on every access,
//! each CSR is updated incrementally in O(1) when a new branch outcome is
//! pushed into the GHR. This is the standard technique from Seznec's TAGE papers.

use super::{BranchPredictor, Ghr, btb::Btb, ras::Ras};
use crate::config::TageConfig;

// ═══════════════════════════════════════════════════════════════════
// Folded History (Circular Shift Register)
// ═══════════════════════════════════════════════════════════════════

/// A single Circular Shift Register for incremental folded XOR computation.
///
/// Maintains a `fold_width`-bit value that represents the XOR-fold of the
/// most recent `hist_length` bits of the GHR, updated incrementally in O(1)
/// per branch outcome push.
#[derive(Clone, Copy, Debug)]
struct FoldedHistory {
    /// Current folded value (only the low `fold_width` bits are meaningful).
    val: u64,
    /// Number of bits to fold into (e.g., `table_bits` for index, `tag_width` for tag).
    fold_width: usize,
    /// History length for this bank (how many GHR bits contribute).
    hist_length: usize,
}

impl FoldedHistory {
    fn new(fold_width: usize, hist_length: usize) -> Self {
        Self { val: 0, fold_width, hist_length }
    }

    /// Incrementally updates the CSR when a new bit is pushed into the GHR.
    ///
    /// Must be called BEFORE `ghr.push()`. `old_bit` is `ghr.bit(hist_length - 1)`,
    /// the bit about to leave this bank's history window.
    ///
    /// Formula: `CSR = rotate_left(CSR, 1, fold_width) ^ new_bit ^ (old_bit << (hist_length % fold_width))`
    #[inline]
    fn update(&mut self, new_bit: bool, old_bit: bool) {
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
    fn recompute(&mut self, ghr: &Ghr) {
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

// ═══════════════════════════════════════════════════════════════════
// TAGE Predictor
// ═══════════════════════════════════════════════════════════════════

/// An entry in a TAGE bank.
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    /// Tag for matching the history/PC hash.
    tag: u16,
    /// 3-bit saturating counter for prediction.
    ctr: i8,
    /// 2-bit useful counter for replacement policy.
    u: u8,
}

/// An entry in the loop predictor table.
#[derive(Clone, Copy, Debug, Default)]
struct LoopEntry {
    /// PC-derived tag for matching.
    tag: u16,
    /// Current iteration count within the loop.
    current_iter: u16,
    /// Learned total trip count (iterations before exit).
    trip_count: u16,
    /// Confidence: number of confirmed full-trip matches (0-3).
    confidence: u8,
    /// Age counter for replacement (higher = more recently used).
    age: u8,
}

/// TAGE Predictor structure with integrated loop predictor.
#[derive(Debug)]
pub struct TagePredictor {
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,

    /// Speculative wide Global History Register — updated at fetch time by
    /// `speculate()` and restored at execute time by `repair_history()`.
    spec_ghr: Ghr,
    /// Committed wide Global History Register — updated only at commit time
    /// in `update_branch()` with the actual branch outcome.
    commit_ghr: Ghr,

    /// Base bimodal predictor table.
    base: Vec<i8>,
    /// Tagged component banks.
    banks: Vec<Vec<TageEntry>>,

    /// Geometric history lengths for each bank.
    hist_lengths: Vec<usize>,
    /// Tag widths for each bank.
    tag_widths: Vec<usize>,
    /// Number of bits for table indexing (log2 of table size).
    table_bits: usize,
    /// Mask for indexing the tables.
    table_mask: usize,

    /// Speculative CSRs for index computation (one per bank, fold_width = table_bits).
    spec_idx_csr: Vec<FoldedHistory>,
    /// Speculative CSRs for index computation (one per bank, fold_width = table_bits - 1).
    spec_idx_csr2: Vec<FoldedHistory>,
    /// Speculative CSRs for tag computation (one per bank, fold_width = tag_widths[i]).
    spec_tag_csr: Vec<FoldedHistory>,
    /// Speculative CSRs for tag computation (one per bank, fold_width = tag_widths[i] - 1).
    spec_tag_csr2: Vec<FoldedHistory>,

    /// Index of the bank providing the current prediction.
    provider_bank: usize,
    /// Index of the alternative bank.
    alt_bank: usize,

    /// Counter for periodic reset of useful bits.
    clock_counter: u32,
    /// Interval for resetting useful bits.
    reset_interval: u32,

    /// Loop predictor table.
    loop_table: Vec<LoopEntry>,
    /// Size of the loop table (number of entries).
    loop_table_size: usize,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    ///
    /// # Panics
    ///
    /// Panics if `config.table_size` is not a power of two, or if `history_lengths`
    /// and `tag_widths` have different lengths.
    pub fn new(config: &TageConfig, btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        assert!(config.table_size.is_power_of_two(), "TAGE table size must be power of 2");

        let num_banks = config.num_banks;
        let hist_lengths = &config.history_lengths;
        let tag_widths = &config.tag_widths;

        assert_eq!(
            hist_lengths.len(),
            num_banks,
            "TAGE: history_lengths.len() ({}) must match num_banks ({num_banks})",
            hist_lengths.len(),
        );
        assert_eq!(
            tag_widths.len(),
            num_banks,
            "TAGE: tag_widths.len() ({}) must match num_banks ({num_banks})",
            tag_widths.len(),
        );

        let table_bits = config.table_size.trailing_zeros() as usize;
        let max_hist = *hist_lengths.iter().max().unwrap_or(&64);

        assert!(
            max_hist <= 1024,
            "TAGE: max history length {max_hist} exceeds GHR capacity of 1024 bits. \
             Reduce your history_lengths config.",
        );

        let mut banks = Vec::with_capacity(num_banks);
        for _ in 0..num_banks {
            banks.push(vec![TageEntry::default(); config.table_size]);
        }

        // Initialize CSR arrays.
        let spec_idx_csr: Vec<_> =
            hist_lengths.iter().map(|&hl| FoldedHistory::new(table_bits, hl)).collect();
        let spec_idx_csr2: Vec<_> = hist_lengths
            .iter()
            .map(|&hl| FoldedHistory::new(table_bits.saturating_sub(1).max(1), hl))
            .collect();
        let spec_tag_csr: Vec<_> = hist_lengths
            .iter()
            .zip(tag_widths.iter())
            .map(|(&hl, &tw)| FoldedHistory::new(tw, hl))
            .collect();
        let spec_tag_csr2: Vec<_> = hist_lengths
            .iter()
            .zip(tag_widths.iter())
            .map(|(&hl, &tw)| FoldedHistory::new(tw.saturating_sub(1).max(1), hl))
            .collect();

        let loop_size = config.loop_table_size.max(1);

        Self {
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            base: vec![0; config.table_size],
            banks,
            hist_lengths: hist_lengths.clone(),
            tag_widths: tag_widths.clone(),
            table_bits,
            table_mask: config.table_size - 1,

            spec_idx_csr,
            spec_idx_csr2,
            spec_tag_csr,
            spec_tag_csr2,

            provider_bank: 0,
            alt_bank: 0,
            clock_counter: 0,
            reset_interval: config.reset_interval,

            loop_table: vec![LoopEntry::default(); loop_size],
            loop_table_size: loop_size,
        }
    }

    // ─── Index / tag using live speculative CSRs (O(1)) ────────────

    /// Computes the table index for a bank using pre-computed speculative CSRs.
    #[inline]
    fn spec_index(&self, pc: u64, bank: usize) -> usize {
        let pc_hash = pc >> 2;
        let h1 = self.spec_idx_csr[bank].val;
        let h2 = self.spec_idx_csr2[bank].val;
        (pc_hash as usize ^ h1 as usize ^ h2 as usize) & self.table_mask
    }

    /// Computes the tag for a bank using pre-computed speculative CSRs.
    #[inline]
    fn spec_tag(&self, pc: u64, bank: usize) -> u16 {
        let width = self.tag_widths[bank];
        let pc_hash = (pc >> 2) ^ (pc >> 18);
        let h1 = self.spec_tag_csr[bank].val;
        let h2 = self.spec_tag_csr2[bank].val;
        ((pc_hash as usize ^ h1 as usize ^ h2 as usize) & ((1 << width) - 1)) as u16
    }

    // ─── Index / tag from a GHR snapshot (O(hist_length) recompute) ───

    /// Computes the table index for a bank by recomputing CSRs from a GHR snapshot.
    /// Used at commit time in `update_branch`.
    fn snapshot_index(&self, pc: u64, bank: usize, ghr: &Ghr) -> usize {
        let mut csr1 = FoldedHistory::new(self.table_bits, self.hist_lengths[bank]);
        csr1.recompute(ghr);
        let mut csr2 =
            FoldedHistory::new(self.table_bits.saturating_sub(1).max(1), self.hist_lengths[bank]);
        csr2.recompute(ghr);
        let pc_hash = pc >> 2;
        (pc_hash as usize ^ csr1.val as usize ^ csr2.val as usize) & self.table_mask
    }

    /// Computes the tag for a bank by recomputing CSRs from a GHR snapshot.
    /// Used at commit time in `update_branch`.
    fn snapshot_tag(&self, pc: u64, bank: usize, ghr: &Ghr) -> u16 {
        let width = self.tag_widths[bank];
        let mut csr1 = FoldedHistory::new(width, self.hist_lengths[bank]);
        csr1.recompute(ghr);
        let mut csr2 = FoldedHistory::new(width.saturating_sub(1).max(1), self.hist_lengths[bank]);
        csr2.recompute(ghr);
        let pc_hash = (pc >> 2) ^ (pc >> 18);
        ((pc_hash as usize ^ csr1.val as usize ^ csr2.val as usize) & ((1 << width) - 1)) as u16
    }

    // ─── Loop predictor ────────────────────────────────────────────

    /// Loop predictor index from PC.
    const fn loop_index(&self, pc: u64) -> usize {
        ((pc >> 2) as usize) % self.loop_table_size
    }

    /// Loop predictor tag from PC (10-bit).
    const fn loop_tag(pc: u64) -> u16 {
        ((pc >> 2) ^ (pc >> 12)) as u16 & 0x3FF
    }

    /// Queries the loop predictor. Returns `Some(taken)` if the loop predictor
    /// has high confidence, otherwise `None`.
    fn loop_predict(&self, pc: u64) -> Option<bool> {
        let idx = self.loop_index(pc);
        let entry = &self.loop_table[idx];
        let tag = Self::loop_tag(pc);

        if entry.tag != tag || entry.confidence < 2 || entry.trip_count == 0 {
            return None;
        }

        if entry.current_iter + 1 >= entry.trip_count {
            Some(false) // loop exit
        } else {
            Some(true) // loop body
        }
    }

    /// Updates the loop predictor with actual branch outcome.
    fn loop_update(&mut self, pc: u64, taken: bool) {
        let idx = self.loop_index(pc);
        let tag = Self::loop_tag(pc);
        let entry = &mut self.loop_table[idx];

        if entry.tag == tag {
            if taken {
                entry.current_iter = entry.current_iter.saturating_add(1);
                entry.age = entry.age.saturating_add(1).min(3);
            } else {
                // Loop exit.
                if entry.trip_count == 0 {
                    entry.trip_count = entry.current_iter;
                    entry.confidence = 1;
                } else if entry.current_iter == entry.trip_count {
                    entry.confidence = entry.confidence.saturating_add(1).min(3);
                } else {
                    entry.trip_count = entry.current_iter;
                    entry.confidence = 0;
                }
                entry.current_iter = 0;
            }
        } else if taken {
            if entry.age == 0 || entry.tag == 0 {
                *entry = LoopEntry { tag, current_iter: 1, trip_count: 0, confidence: 0, age: 1 };
            } else {
                entry.age = entry.age.saturating_sub(1);
            }
        }
    }

    /// Recomputes all speculative CSRs from the current spec_ghr.
    fn recompute_all_csrs(&mut self) {
        let ghr = &self.spec_ghr;
        for i in 0..self.banks.len() {
            self.spec_idx_csr[i].recompute(ghr);
            self.spec_idx_csr2[i].recompute(ghr);
            self.spec_tag_csr[i].recompute(ghr);
            self.spec_tag_csr2[i].recompute(ghr);
        }
    }
}

impl BranchPredictor for TagePredictor {
    /// Predicts branch direction and target.
    ///
    /// Searches the tagged banks for the longest history match (provider).
    /// If no match is found, uses the base predictor. The loop predictor
    /// can override when it has high confidence.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        // Check loop predictor first — high-confidence override.
        if let Some(loop_taken) = self.loop_predict(pc) {
            return (loop_taken, if loop_taken { self.btb.lookup(pc) } else { None });
        }

        let mut provider = 0;
        let num_banks = self.banks.len();

        for i in (0..num_banks).rev() {
            let idx = self.spec_index(pc, i);
            let tag = self.spec_tag(pc, i);
            if self.banks[i][idx].tag == tag {
                provider = i + 1;
                break;
            }
        }

        if provider > 0 {
            let bank_idx = provider - 1;
            let idx = self.spec_index(pc, bank_idx);
            let ctr = self.banks[bank_idx][idx].ctr;
            return (ctr >= 0, self.btb.lookup(pc));
        }

        let base_idx = ((pc >> 2) as usize) & self.table_mask;
        (self.base[base_idx] >= 0, self.btb.lookup(pc))
    }

    /// Updates the predictor state at commit time.
    ///
    /// Uses the GHR snapshot from prediction time (not the live speculative
    /// GHR) to ensure we train the same table entries that were consulted
    /// during prediction. CSRs are recomputed from the snapshot via
    /// `snapshot_index`/`snapshot_tag`.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr) {
        // Update loop predictor.
        self.loop_update(pc, taken);

        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for bank in &mut self.banks {
                for entry in bank {
                    entry.u >>= 1;
                }
            }
        }

        let mut provider = 0;
        let mut alt = 0;
        let num_banks = self.banks.len();

        for i in (0..num_banks).rev() {
            let idx = self.snapshot_index(pc, i, ghr_snapshot);
            let tag = self.snapshot_tag(pc, i, ghr_snapshot);
            if self.banks[i][idx].tag == tag {
                if provider == 0 {
                    provider = i + 1;
                } else if alt == 0 {
                    alt = i + 1;
                    break;
                }
            }
        }
        self.provider_bank = provider;
        self.alt_bank = alt;

        let pred_taken = if self.provider_bank > 0 {
            let idx = self.snapshot_index(pc, self.provider_bank - 1, ghr_snapshot);
            self.banks[self.provider_bank - 1][idx].ctr >= 0
        } else {
            let base_idx = ((pc >> 2) as usize) & self.table_mask;
            self.base[base_idx] >= 0
        };

        let alt_taken = if self.alt_bank > 0 {
            let idx = self.snapshot_index(pc, self.alt_bank - 1, ghr_snapshot);
            self.banks[self.alt_bank - 1][idx].ctr >= 0
        } else {
            let base_idx = ((pc >> 2) as usize) & self.table_mask;
            self.base[base_idx] >= 0
        };

        let mispredicted = pred_taken != taken;

        if self.provider_bank > 0 {
            let bank_idx = self.provider_bank - 1;
            let idx = self.snapshot_index(pc, bank_idx, ghr_snapshot);
            let e = &mut self.banks[bank_idx][idx];

            if taken {
                if e.ctr < 3 {
                    e.ctr += 1;
                }
            } else if e.ctr > -4 {
                e.ctr -= 1;
            }

            if !mispredicted && (alt_taken != taken) && e.u < 3 {
                e.u += 1;
            }
            if mispredicted && e.u > 0 {
                e.u -= 1;
            }
        } else {
            let base_idx = ((pc >> 2) as usize) & self.table_mask;
            let b = &mut self.base[base_idx];
            if taken {
                if *b < 1 {
                    *b += 1;
                }
            } else if *b > -2 {
                *b -= 1;
            }
        }

        if mispredicted {
            let start_bank = if self.provider_bank == 0 { 0 } else { self.provider_bank };

            if start_bank < num_banks {
                let mut allocated = false;
                for i in start_bank..num_banks {
                    let idx = self.snapshot_index(pc, i, ghr_snapshot);
                    let tag = self.snapshot_tag(pc, i, ghr_snapshot);
                    let e = &mut self.banks[i][idx];

                    if e.u == 0 {
                        e.tag = tag;
                        e.ctr = if taken { 0 } else { -1 };
                        e.u = 1;
                        allocated = true;
                        break;
                    }
                }

                if !allocated {
                    for i in start_bank..num_banks {
                        let idx = self.snapshot_index(pc, i, ghr_snapshot);
                        if self.banks[i][idx].u > 0 {
                            self.banks[i][idx].u -= 1;
                        }
                    }
                }
            }
        }

        self.commit_ghr.push(taken);

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
        let _ = self.ras.pop();
    }

    /// Speculatively updates the GHR and all CSRs with a predicted branch outcome.
    ///
    /// O(num_banks) — each CSR is updated in O(1).
    fn speculate(&mut self, _pc: u64, taken: bool) {
        // Read old bits BEFORE push — these are the bits leaving each bank's window.
        for i in 0..self.banks.len() {
            let hl = self.hist_lengths[i];
            let old_bit = if hl > 0 { self.spec_ghr.bit(hl - 1) } else { false };
            self.spec_idx_csr[i].update(taken, old_bit);
            self.spec_idx_csr2[i].update(taken, old_bit);
            self.spec_tag_csr[i].update(taken, old_bit);
            self.spec_tag_csr2[i].update(taken, old_bit);
        }
        self.spec_ghr.push(taken);
    }

    fn snapshot_history(&self) -> Ghr {
        self.spec_ghr // Copy
    }

    /// Restores the GHR and recomputes all CSRs from the snapshot.
    fn repair_history(&mut self, ghr: &Ghr) {
        self.spec_ghr = *ghr;
        self.recompute_all_csrs();
    }

    fn snapshot_ras(&self) -> usize {
        self.ras.snapshot_ptr()
    }

    fn restore_ras(&mut self, ptr: usize) {
        self.ras.restore_ptr(ptr);
    }

    fn update_btb(&mut self, pc: u64, target: u64) {
        self.btb.update(pc, target);
    }

    fn repair_to_committed(&mut self) {
        self.spec_ghr = self.commit_ghr;
        self.recompute_all_csrs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TageConfig {
        TageConfig {
            num_banks: 4,
            table_size: 256,
            loop_table_size: 16,
            reset_interval: 100_000,
            history_lengths: vec![5, 15, 44, 130],
            tag_widths: vec![9, 9, 10, 10],
        }
    }

    #[test]
    fn test_folded_history_update_matches_recompute() {
        // Push N bits incrementally, then verify the CSR matches a fresh recompute.
        let hist_length = 44;
        let fold_width = 11;
        let mut ghr = Ghr::with_len(hist_length);
        let mut csr = FoldedHistory::new(fold_width, hist_length);

        for i in 0..200 {
            let bit = (i * 7 + 3) % 2 == 0; // deterministic pseudo-random pattern
            let old_bit = if hist_length > 0 { ghr.bit(hist_length - 1) } else { false };
            csr.update(bit, old_bit);
            ghr.push(bit);
        }

        // Recompute from scratch and compare.
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
    fn test_spec_index_matches_snapshot_index() {
        let config = test_config();
        let mut tage = TagePredictor::new(&config, 64, 4, 8);
        let pc: u64 = 0x8000_1234;

        // Push some history.
        for i in 0u64..50 {
            let taken = i % 3 != 0;
            tage.speculate(pc.wrapping_add(i * 4), taken);
        }

        // Snapshot the current state.
        let snapshot = tage.snapshot_history();

        // Verify spec_index/tag match snapshot_index/tag for all banks.
        for bank in 0..config.num_banks {
            let si = tage.spec_index(pc, bank);
            let ri = tage.snapshot_index(pc, bank, &snapshot);
            assert_eq!(si, ri, "Index mismatch for bank {bank}");

            let st = tage.spec_tag(pc, bank);
            let rt = tage.snapshot_tag(pc, bank, &snapshot);
            assert_eq!(st, rt, "Tag mismatch for bank {bank}");
        }
    }

    #[test]
    fn test_repair_history_restores_csrs() {
        let config = test_config();
        let mut tage = TagePredictor::new(&config, 64, 4, 8);
        let pc: u64 = 0x8000_1000;

        // Push some history and take a snapshot.
        for i in 0u64..20 {
            tage.speculate(pc.wrapping_add(i * 4), i % 2 == 0);
        }
        let snapshot = tage.snapshot_history();

        // Save CSR values at snapshot point.
        let saved_idx: Vec<u64> = tage.spec_idx_csr.iter().map(|c| c.val).collect();
        let saved_tag: Vec<u64> = tage.spec_tag_csr.iter().map(|c| c.val).collect();

        // Push more speculative history (diverge from snapshot).
        for i in 0u64..30 {
            tage.speculate(pc.wrapping_add((20 + i) * 4), true);
        }

        // Repair to the snapshot.
        tage.repair_history(&snapshot);

        // CSRs should match the saved values.
        let restored_idx: Vec<u64> = tage.spec_idx_csr.iter().map(|c| c.val).collect();
        let restored_tag: Vec<u64> = tage.spec_tag_csr.iter().map(|c| c.val).collect();
        assert_eq!(saved_idx, restored_idx, "Index CSRs not restored correctly");
        assert_eq!(saved_tag, restored_tag, "Tag CSRs not restored correctly");
    }
}
