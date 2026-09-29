//! TAGE's speculative global and path histories.
//!
//! They are kept as gem5's `TAGEBase` keeps them: a circular buffer of
//! history bits with a head pointer, the folded histories each tagged table
//! hashes, and a path history.
//!
//! Every control instruction shifts bits in. A [`HistoryCheckpoint`] taken
//! before an instruction's update holds the head, the path and the folds,
//! so a squash restores them exactly without recomputing any fold; the bits
//! behind the head are never overwritten while a checkpoint can return to
//! them, because the buffer holds far more than the longest history plus
//! every bit the instructions in flight can push.

use super::folded_history::FoldedHistory;
use super::tagged_bank::MAX_BANKS;
use crate::config::TageHistoryMode;
use crate::uarch::bpred::direction::Jump;

/// History bits the buffer holds beyond the longest history: more than
/// every in-flight control instruction can push before a squash rewinds.
const IN_FLIGHT_MARGIN: usize = 1 << 13;

/// A control instruction as it shifts the histories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryBranch {
    /// A conditional branch.
    Conditional,
    /// A direct jump.
    DirectJump,
    /// An indirect jump.
    IndirectJump,
}

impl From<Jump> for HistoryBranch {
    fn from(jump: Jump) -> Self {
        match jump {
            Jump::Direct => Self::DirectJump,
            Jump::Indirect => Self::IndirectJump,
        }
    }
}

/// The histories as they were before one control instruction's update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCheckpoint {
    head: usize,
    path: u32,
    folds: [[u32; 3]; MAX_BANKS],
}

/// TAGE's global history buffer, folded histories and path history.
#[derive(Debug)]
pub struct TageHistories {
    buffer: Vec<bool>,
    /// Where the youngest bit is.
    head: usize,
    path: u32,
    path_mask: u32,
    mode: TageHistoryMode,
    num_banks: usize,
    hist_lengths: [usize; MAX_BANKS],
    /// Per bank: the index fold, the tag fold, and the tag fold a bit
    /// narrower (gem5's `computeIndices`, `computeTags[0]`, `computeTags[1]`).
    folds: [[FoldedHistory; 3]; MAX_BANKS],
}

impl TageHistories {
    /// Histories for tagged tables with `hist_lengths` and `tag_widths`,
    /// indexed by `table_bits`.
    pub fn new(
        hist_lengths: &[usize],
        tag_widths: &[usize],
        table_bits: usize,
        path_bits: u32,
        mode: TageHistoryMode,
    ) -> Self {
        let mut lengths = [0; MAX_BANKS];
        let mut folds = [[FoldedHistory::new(0, 0); 3]; MAX_BANKS];
        for (bank, (&length, &tag_width)) in hist_lengths.iter().zip(tag_widths).enumerate() {
            lengths[bank] = length;
            folds[bank] = [
                FoldedHistory::new(table_bits, length),
                FoldedHistory::new(tag_width, length),
                FoldedHistory::new(tag_width.saturating_sub(1).max(1), length),
            ];
        }
        let longest = hist_lengths.iter().copied().max().unwrap_or(0);
        Self {
            buffer: vec![false; (longest + IN_FLIGHT_MARGIN).next_power_of_two()],
            head: 0,
            path: 0,
            path_mask: ((1u64 << path_bits) - 1) as u32,
            mode,
            num_banks: hist_lengths.len(),
            hist_lengths: lengths,
            folds,
        }
    }

    /// The history bit `age` instructions' bits ago; 0 is the youngest.
    fn bit(&self, age: usize) -> bool {
        self.buffer[self.head.wrapping_sub(age) & (self.buffer.len() - 1)]
    }

    fn push(&mut self, bit: bool) {
        for bank in 0..self.num_banks {
            let length = self.hist_lengths[bank];
            let leaving = length > 0 && self.bit(length - 1);
            for fold in &mut self.folds[bank] {
                fold.update(bit, leaving);
            }
        }
        self.head = (self.head + 1) & (self.buffer.len() - 1);
        self.buffer[self.head] = bit;
    }

    /// Shifts in the instruction at `pc` going `taken`: `TAGEBase` pushes
    /// its direction and one PC bit of path; TAGE-SC-L pushes two PC-hashed
    /// bits (three for an indirect jump) and a path bit with each.
    pub fn speculate(&mut self, pc: u64, taken: bool, branch: HistoryBranch) {
        match self.mode {
            TageHistoryMode::Direction => {
                self.push(taken);
                self.path = ((self.path << 1) | ((pc >> 2) & 1) as u32) & self.path_mask;
            }
            TageHistoryMode::PcBits => {
                let mut bits = (pc ^ (pc >> 2)) ^ u64::from(taken);
                let mut path = pc ^ (pc >> 2) ^ (pc >> 4);
                let count = if branch == HistoryBranch::IndirectJump { 3 } else { 2 };
                for _ in 0..count {
                    self.push(bits & 1 != 0);
                    bits >>= 1;
                    self.path = ((self.path << 1) ^ (path & 127) as u32) & self.path_mask;
                    path >>= 1;
                }
            }
        }
    }

    /// The state an instruction's update starts from.
    pub fn checkpoint(&self) -> HistoryCheckpoint {
        let mut folds = [[0; 3]; MAX_BANKS];
        for (saved, bank) in folds.iter_mut().zip(&self.folds).take(self.num_banks) {
            for (value, fold) in saved.iter_mut().zip(bank) {
                *value = fold.val as u32;
            }
        }
        HistoryCheckpoint { head: self.head, path: self.path, folds }
    }

    /// Returns to `checkpoint`.
    pub fn restore(&mut self, checkpoint: &HistoryCheckpoint) {
        self.head = checkpoint.head;
        self.path = checkpoint.path;
        for (bank, saved) in self.folds.iter_mut().zip(&checkpoint.folds).take(self.num_banks) {
            for (fold, &value) in bank.iter_mut().zip(saved) {
                fold.val = u64::from(value);
            }
        }
    }

    /// The path history.
    pub const fn path(&self) -> u32 {
        self.path
    }

    /// `bank`'s index fold, tag fold and narrower tag fold.
    pub const fn folds(&self, bank: usize) -> [u64; 3] {
        let [index, tag, short_tag] = &self.folds[bank];
        [index.val, tag.val, short_tag.val]
    }

    /// Sets the path history, for tests that compare paths.
    #[cfg(test)]
    pub const fn set_path(&mut self, path: u32) {
        self.path = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn histories(mode: TageHistoryMode) -> TageHistories {
        TageHistories::new(&[5, 15, 44, 130], &[9, 9, 10, 10], 8, 16, mode)
    }

    /// Each fold as recomputed from the buffer bit by bit.
    fn recomputed(h: &TageHistories, bank: usize) -> [u64; 3] {
        let mut folds = h.folds[bank];
        for fold in &mut folds {
            fold.val = 0;
        }
        for age in (0..h.hist_lengths[bank]).rev() {
            for fold in &mut folds {
                fold.update(h.bit(age), false);
            }
        }
        folds.map(|fold| fold.val)
    }

    #[test]
    fn the_folds_track_the_buffer() {
        let mut h = histories(TageHistoryMode::Direction);

        for i in 0u64..400 {
            h.speculate(0x8000_0000 + 4 * i, i % 3 != 0, HistoryBranch::Conditional);
        }

        for bank in 0..4 {
            assert_eq!(h.folds(bank), recomputed(&h, bank), "bank {bank}");
        }
    }

    #[test]
    fn restoring_a_checkpoint_undoes_every_later_update() {
        let mut h = histories(TageHistoryMode::PcBits);
        for i in 0u64..100 {
            h.speculate(0x8000_0000 + 4 * i, i % 2 == 0, HistoryBranch::Conditional);
        }
        let checkpoint = h.checkpoint();
        let before = ((0..4).map(|bank| h.folds(bank)).collect::<Vec<_>>(), h.path());

        for i in 0u64..300 {
            h.speculate(0x8000_4000 + 4 * i, true, HistoryBranch::IndirectJump);
        }
        h.restore(&checkpoint);

        assert_eq!(((0..4).map(|bank| h.folds(bank)).collect::<Vec<_>>(), h.path()), before);
        for bank in 0..4 {
            assert_eq!(h.folds(bank), recomputed(&h, bank), "bank {bank}");
        }
    }

    #[test]
    fn pc_bit_history_pushes_three_bits_for_an_indirect_jump_and_two_otherwise() {
        let mut h = histories(TageHistoryMode::PcBits);
        let start = h.head;

        h.speculate(0x8000_0000, true, HistoryBranch::Conditional);
        h.speculate(0x8000_0010, true, HistoryBranch::DirectJump);
        h.speculate(0x8000_0020, true, HistoryBranch::IndirectJump);

        assert_eq!(h.head - start, 7);
    }

    #[test]
    fn the_path_keeps_its_configured_width() {
        let mut h = TageHistories::new(&[5], &[9], 8, 27, TageHistoryMode::PcBits);

        for i in 0u64..100 {
            h.speculate(0x8765_4320 + 4 * i, true, HistoryBranch::Conditional);
        }

        assert_eq!(h.path() >> 27, 0);
    }
}
