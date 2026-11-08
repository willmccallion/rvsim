//! Shared geometric bank infrastructure for TAGE-family predictors.
//!
//! Manages N sets of `FoldedHistory` CSRs for geometric-history tagged tables.
//! Handles speculative update, snapshot recompute, and repair. Provides indexing
//! and CSR infrastructure only — entry types are owned by consumers.
//!
//! All per-bank metadata uses fixed-size arrays (`[T; MAX_BANKS]`) to keep the
//! hot prediction/speculation path free of heap indirection and bounds checks.

use super::folded_history::FoldedHistory;
use crate::config::MAX_TAGE_BANKS;
use crate::uarch::bpred::Ghr;

/// Maximum number of banks supported: [`crate::config::MAX_TAGE_BANKS`].
pub const MAX_BANKS: usize = MAX_TAGE_BANKS;

/// Manages N sets of `FoldedHistory` CSRs for geometric-history tagged tables.
///
/// This is the core primitive shared by TAGE (direction) and ITTAGE (indirect target).
/// Each bank has four CSRs: two for index computation, two for tag computation.
///
/// All per-bank arrays are fixed-size `[T; MAX_BANKS]` — only the first
/// `num_banks` elements are meaningful. This eliminates heap indirection on the
/// hot speculate/predict path.
#[derive(Debug)]
pub struct GeoBankSet {
    /// Number of active banks.
    num_banks: usize,
    /// Geometric history lengths for each bank.
    hist_lengths: [usize; MAX_BANKS],
    /// Tag widths for each bank.
    tag_widths: [usize; MAX_BANKS],
    /// Number of bits for table indexing (log2 of table size).
    table_bits: usize,
    /// Mask for indexing the tables.
    table_mask: usize,

    idx_csr: [FoldedHistory; MAX_BANKS],
    idx_csr2: [FoldedHistory; MAX_BANKS],
    tag_csr: [FoldedHistory; MAX_BANKS],
    tag_csr2: [FoldedHistory; MAX_BANKS],

    /// Mirrors of the speculative CSRs, advanced incrementally via
    /// `update_committed_csrs()` at commit time.
    committed_idx_csr: [FoldedHistory; MAX_BANKS],
    committed_idx_csr2: [FoldedHistory; MAX_BANKS],
    committed_tag_csr: [FoldedHistory; MAX_BANKS],
    committed_tag_csr2: [FoldedHistory; MAX_BANKS],
}

impl GeoBankSet {
    /// Creates a new `GeoBankSet` with the given geometric history lengths and tag widths.
    ///
    /// # Panics
    ///
    /// Panics if `hist_lengths` and `tag_widths` have different lengths, or if
    /// the number of banks exceeds `MAX_BANKS`.
    pub fn new(hist_lengths: &[usize], tag_widths: &[usize], table_bits: usize) -> Self {
        let n = hist_lengths.len();
        assert_eq!(
            n,
            tag_widths.len(),
            "GeoBankSet: hist_lengths.len() ({n}) must match tag_widths.len() ({})",
            tag_widths.len(),
        );
        assert!(n <= MAX_BANKS, "GeoBankSet: {n} banks exceeds MAX_BANKS ({MAX_BANKS})");

        let mut hl = [0usize; MAX_BANKS];
        let mut tw = [0usize; MAX_BANKS];
        let mut idx_csr = [FoldedHistory::new(0, 0); MAX_BANKS];
        let mut idx_csr2 = [FoldedHistory::new(0, 0); MAX_BANKS];
        let mut tag_csr = [FoldedHistory::new(0, 0); MAX_BANKS];
        let mut tag_csr2 = [FoldedHistory::new(0, 0); MAX_BANKS];

        for i in 0..n {
            hl[i] = hist_lengths[i];
            tw[i] = tag_widths[i];
            idx_csr[i] = FoldedHistory::new(table_bits, hist_lengths[i]);
            idx_csr2[i] = FoldedHistory::new(table_bits.saturating_sub(1).max(1), hist_lengths[i]);
            tag_csr[i] = FoldedHistory::new(tag_widths[i], hist_lengths[i]);
            tag_csr2[i] =
                FoldedHistory::new(tag_widths[i].saturating_sub(1).max(1), hist_lengths[i]);
        }

        Self {
            num_banks: n,
            hist_lengths: hl,
            tag_widths: tw,
            table_bits,
            table_mask: (1 << table_bits) - 1,
            committed_idx_csr: idx_csr,
            committed_idx_csr2: idx_csr2,
            committed_tag_csr: tag_csr,
            committed_tag_csr2: tag_csr2,
            idx_csr,
            idx_csr2,
            tag_csr,
            tag_csr2,
        }
    }

    /// Number of banks in this set.
    #[inline]
    pub const fn num_banks(&self) -> usize {
        self.num_banks
    }

    /// Computes the table index for a bank using pre-computed speculative CSRs. `O(1)`.
    #[inline]
    pub const fn spec_index(&self, pc: u64, bank: usize) -> usize {
        let index = fold_hash(index_pc(pc), self.idx_csr[bank].val, self.idx_csr2[bank].val);
        index as usize & self.table_mask
    }

    /// Computes the tag for a bank using pre-computed speculative CSRs. `O(1)`.
    #[inline]
    pub const fn spec_tag(&self, pc: u64, bank: usize) -> u16 {
        let tag = fold_hash(tag_pc(pc), self.tag_csr[bank].val, self.tag_csr2[bank].val);
        (tag & ((1 << self.tag_widths[bank]) - 1)) as u16
    }

    #[cfg(test)]
    /// Computes the table index for a bank by recomputing CSRs from a GHR snapshot.
    /// `O(hist_length)`. Used at commit time.
    pub fn snapshot_index(&self, pc: u64, bank: usize, ghr: &Ghr) -> usize {
        let mut csr1 = FoldedHistory::new(self.table_bits, self.hist_lengths[bank]);
        csr1.recompute(ghr);
        let mut csr2 =
            FoldedHistory::new(self.table_bits.saturating_sub(1).max(1), self.hist_lengths[bank]);
        csr2.recompute(ghr);
        fold_hash(index_pc(pc), csr1.val, csr2.val) as usize & self.table_mask
    }

    #[cfg(test)]
    /// Computes the tag for a bank by recomputing CSRs from a GHR snapshot.
    /// `O(hist_length)`. Used at commit time.
    pub fn snapshot_tag(&self, pc: u64, bank: usize, ghr: &Ghr) -> u16 {
        let width = self.tag_widths[bank];
        let mut csr1 = FoldedHistory::new(width, self.hist_lengths[bank]);
        csr1.recompute(ghr);
        let mut csr2 = FoldedHistory::new(width.saturating_sub(1).max(1), self.hist_lengths[bank]);
        csr2.recompute(ghr);
        (fold_hash(tag_pc(pc), csr1.val, csr2.val) & ((1 << width) - 1)) as u16
    }

    /// Computes all bank indices and tags in one pass by recomputing CSRs from a
    /// GHR snapshot. Returns `([indices; MAX_BANKS], [tags; MAX_BANKS])`.
    /// Only the first `num_banks` elements are meaningful.
    /// `O(sum of hist_lengths)` but called once instead of per-bank.
    pub fn snapshot_all(&self, pc: u64, ghr: &Ghr) -> ([usize; MAX_BANKS], [u16; MAX_BANKS]) {
        let mut indices = [0usize; MAX_BANKS];
        let mut tags = [0u16; MAX_BANKS];
        for i in 0..self.num_banks {
            let hl = self.hist_lengths[i];
            let tw = self.tag_widths[i];

            let mut csr1 = FoldedHistory::new(self.table_bits, hl);
            csr1.recompute(ghr);
            let mut csr2 = FoldedHistory::new(self.table_bits.saturating_sub(1).max(1), hl);
            csr2.recompute(ghr);
            indices[i] = fold_hash(index_pc(pc), csr1.val, csr2.val) as usize & self.table_mask;

            let mut tcsr1 = FoldedHistory::new(tw, hl);
            tcsr1.recompute(ghr);
            let mut tcsr2 = FoldedHistory::new(tw.saturating_sub(1).max(1), hl);
            tcsr2.recompute(ghr);
            tags[i] = (fold_hash(tag_pc(pc), tcsr1.val, tcsr2.val) & ((1 << tw) - 1)) as u16;
        }

        (indices, tags)
    }

    /// Incrementally updates all CSRs for a new branch outcome. `O(num_banks)`.
    /// Must be called BEFORE `ghr.push()`.
    #[inline]
    pub fn update_csrs(&mut self, taken: bool, ghr: &Ghr) {
        for i in 0..self.num_banks {
            let hl = self.hist_lengths[i];
            let old_bit = if hl > 0 { ghr.bit(hl - 1) } else { false };
            self.idx_csr[i].update(taken, old_bit);
            self.idx_csr2[i].update(taken, old_bit);
            self.tag_csr[i].update(taken, old_bit);
            self.tag_csr2[i].update(taken, old_bit);
        }
    }

    /// Recomputes all speculative CSRs from scratch from the given GHR.
    /// `O(sum of hist_lengths)`. Used after misprediction recovery.
    pub fn recompute_all(&mut self, ghr: &Ghr) {
        for i in 0..self.num_banks {
            self.idx_csr[i].recompute(ghr);
            self.idx_csr2[i].recompute(ghr);
            self.tag_csr[i].recompute(ghr);
            self.tag_csr2[i].recompute(ghr);
        }
    }

    /// Incrementally updates all committed CSRs for a new branch outcome. `O(num_banks)`.
    /// Must be called BEFORE `commit_ghr.push()`.
    #[inline]
    pub fn update_committed_csrs(&mut self, taken: bool, ghr: &Ghr) {
        for i in 0..self.num_banks {
            let hl = self.hist_lengths[i];
            let old_bit = if hl > 0 { ghr.bit(hl - 1) } else { false };
            self.committed_idx_csr[i].update(taken, old_bit);
            self.committed_idx_csr2[i].update(taken, old_bit);
            self.committed_tag_csr[i].update(taken, old_bit);
            self.committed_tag_csr2[i].update(taken, old_bit);
        }
    }
}

/// The PC bits an index starts from.
const fn index_pc(pc: u64) -> u64 {
    pc >> 2
}

/// The PC bits a tag starts from.
const fn tag_pc(pc: u64) -> u64 {
    (pc >> 2) ^ (pc >> 18)
}

/// `pc_bits` hashed with a bank's fold and its one-bit-narrower fold, as
/// Seznec's TAGE forms a tag. A history no longer than both folds sits
/// unfolded in each, so the narrow one is shifted to keep them from
/// cancelling.
const fn fold_hash(pc_bits: u64, fold: u64, narrow_fold: u64) -> u64 {
    pc_bits ^ fold ^ (narrow_fold << 1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Both folds of a 4-bit history hold it unfolded; hashed alike, they
    /// would cancel and leave the bank blind to its history.
    #[test]
    fn a_history_shorter_than_both_folds_still_moves_the_index_and_tag() {
        let banks = GeoBankSet::new(&[4], &[9], 8);
        let pc = 0x8000_1234;
        let mut all_taken = Ghr::with_len(4);
        for _ in 0..4 {
            all_taken.push(true);
        }

        let (taken_indices, taken_tags) = banks.snapshot_all(pc, &all_taken);
        let (empty_indices, empty_tags) = banks.snapshot_all(pc, &Ghr::with_len(4));

        assert_ne!(taken_indices[0], empty_indices[0], "index");
        assert_ne!(taken_tags[0], empty_tags[0], "tag");
    }

    #[test]
    fn speculative_index_and_tag_match_a_snapshot_recompute() {
        let hist_lengths = [5, 15, 44, 130];
        let tag_widths = [9, 9, 10, 10];
        let table_bits = 8;
        let mut banks = GeoBankSet::new(&hist_lengths, &tag_widths, table_bits);
        let max_hist = *hist_lengths.iter().max().unwrap();
        let mut ghr = Ghr::with_len(max_hist);
        let pc: u64 = 0x8000_1234;

        for i in 0u64..50 {
            let taken = i % 3 != 0;
            banks.update_csrs(taken, &ghr);
            ghr.push(taken);
        }

        for bank in 0..4 {
            let si = banks.spec_index(pc, bank);
            let ri = banks.snapshot_index(pc, bank, &ghr);
            assert_eq!(si, ri, "Index mismatch for bank {bank}");

            let st = banks.spec_tag(pc, bank);
            let rt = banks.snapshot_tag(pc, bank, &ghr);
            assert_eq!(st, rt, "Tag mismatch for bank {bank}");
        }
    }

    #[test]
    fn recompute_all_restores_the_csrs_of_a_ghr_snapshot() {
        let hist_lengths = [5, 15, 44, 130];
        let tag_widths = [9, 9, 10, 10];
        let table_bits = 8;
        let mut banks = GeoBankSet::new(&hist_lengths, &tag_widths, table_bits);
        let max_hist = *hist_lengths.iter().max().unwrap();
        let mut ghr = Ghr::with_len(max_hist);
        let pc: u64 = 0x8000_1000;

        // Push some history.
        for i in 0u64..20 {
            let taken = i % 2 == 0;
            banks.update_csrs(taken, &ghr);
            ghr.push(taken);
        }

        let snapshot = ghr;
        let saved_idx: Vec<u64> = banks.idx_csr[..4].iter().map(|c| c.val).collect();
        let saved_tag: Vec<u64> = banks.tag_csr[..4].iter().map(|c| c.val).collect();

        // Push more speculative history.
        for _ in 0u64..30 {
            banks.update_csrs(true, &ghr);
            ghr.push(true);
        }

        // Repair.
        ghr = snapshot;
        banks.recompute_all(&ghr);

        let restored_idx: Vec<u64> = banks.idx_csr[..4].iter().map(|c| c.val).collect();
        let restored_tag: Vec<u64> = banks.tag_csr[..4].iter().map(|c| c.val).collect();
        assert_eq!(saved_idx, restored_idx, "Index CSRs not restored");
        assert_eq!(saved_tag, restored_tag, "Tag CSRs not restored");

        // Verify spec matches snapshot after repair.
        for bank in 0..4 {
            assert_eq!(
                banks.spec_index(pc, bank),
                banks.snapshot_index(pc, bank, &ghr),
                "Post-repair index mismatch for bank {bank}"
            );
        }
    }

    #[test]
    fn snapshot_all_matches_the_per_bank_snapshots() {
        let hist_lengths = [5, 15, 44, 130];
        let tag_widths = [9, 9, 10, 10];
        let table_bits = 8;
        let banks = GeoBankSet::new(&hist_lengths, &tag_widths, table_bits);
        let max_hist = *hist_lengths.iter().max().unwrap();
        let mut ghr = Ghr::with_len(max_hist);

        // Build up some history.
        for i in 0u64..60 {
            ghr.push(i % 3 != 0);
        }

        let pcs = [0x8000_1234u64, 0xDEAD_BEEFu64, 0x0000_0004u64];
        for &pc in &pcs {
            let (indices, tags) = banks.snapshot_all(pc, &ghr);
            for bank in 0..4 {
                assert_eq!(
                    indices[bank],
                    banks.snapshot_index(pc, bank, &ghr),
                    "snapshot_all index mismatch for PC {pc:#x} bank {bank}"
                );
                assert_eq!(
                    tags[bank],
                    banks.snapshot_tag(pc, bank, &ghr),
                    "snapshot_all tag mismatch for PC {pc:#x} bank {bank}"
                );
            }
        }
    }
}
