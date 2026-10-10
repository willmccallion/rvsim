//! Rocket's branch history table.
//!
//! A table of one- or two-bit counters indexed by the fetch PC with a hash
//! of the global history placed above the PC bits, so nearby branches never
//! share a counter however their histories differ (`BHT` in rocket-chip's
//! `rocket/BTB.scala`). The history keeps its newest outcome in its top
//! bit; it advances with each predicted branch, is rebuilt from a
//! mispredicted branch's fetch-time history and real direction, and is
//! reset to a mispredicted jump's fetch-time history. A counter trains on
//! every retired branch from the value it was read with. Rocket predicts
//! only when its BTB hits; here every conditional branch is predicted,
//! which differs only before the BTB has learned a branch.

use crate::config::BhtConfig;
use crate::uarch::bpred::direction::{BranchClass, DirectionPredictor, Jump, Retired};

/// Bytes a Rocket fetch returns (`fetchBytes`): two compressed slots, 4 bytes.
const FETCH_BYTES_LOG2: u32 = 2;

/// Rocket's BHT.
#[derive(Debug)]
pub struct BhtPredictor {
    table: Vec<u8>,
    /// The speculative global history, newest outcome in its top bit.
    history: u64,
    index_bits: u32,
    counter_bits: u32,
    history_length: u32,
    history_bits: u32,
    /// `floor(sqrt(3) / 2 * 2^history_length)`, which the history is
    /// multiplied by to hash it.
    history_multiplier: u64,
}

/// What a BHT prediction read: the history and counter at the time, and
/// whether the instruction was a conditional branch.
#[derive(Clone, Copy, Debug)]
pub struct BhtHistory {
    history: u64,
    value: u8,
    class: BranchClass,
}

impl BhtPredictor {
    /// A BHT with `config`'s table and history; the configuration has been
    /// validated.
    #[must_use]
    pub fn new(config: &BhtConfig) -> Self {
        let history_multiplier =
            (3f64.sqrt() / 2.0 * 2f64.powi(config.history_length.cast_signed())) as u64;
        Self {
            table: vec![0; config.entries],
            history: 0,
            index_bits: config.index_bits(),
            counter_bits: config.counter_bits,
            history_length: config.history_length,
            history_bits: config.history_bits,
            history_multiplier,
        }
    }

    #[cfg(test)]
    /// The speculative global history.
    pub const fn history(&self) -> u64 {
        self.history
    }

    /// The counter for the branch at `pc` under `history` (`BHT.index`):
    /// the fetch address's low index bits, folded with the two above them,
    /// XOR the hashed history placed above the PC bits.
    #[must_use]
    pub(crate) const fn index(&self, pc: u64, history: u64) -> usize {
        let fetch_address = pc >> FETCH_BYTES_LOG2;
        let address = (fetch_address & low_bits(self.index_bits))
            ^ ((fetch_address >> self.index_bits) & 0b11);
        let history = self.hashed_history(history) << (self.index_bits - self.history_bits);
        ((address ^ history) & low_bits(self.index_bits)) as usize
    }

    /// The top `history_bits` of the history multiplied by the
    /// multiplier, within the history's width (`hashHistory`).
    const fn hashed_history(&self, history: u64) -> u64 {
        if self.history_length == self.history_bits {
            return history;
        }
        let product = self.history_multiplier.wrapping_mul(history) & low_bits(self.history_length);
        product >> (self.history_length - self.history_bits)
    }

    /// `history` with `taken` shifted in at the top.
    const fn shifted(&self, history: u64, taken: bool) -> u64 {
        ((taken as u64) << (self.history_length - 1)) | (history >> 1)
    }

    /// The counter after a branch read as `value` resolved `taken`
    /// (`updateTable`).
    const fn trained(&self, value: u8, taken: bool) -> u8 {
        if self.counter_bits == 1 {
            return taken as u8;
        }
        let direction = (taken ^ (value & 1 == 1)) as u8;
        let strength = (value == 1 || (value & 0b10 != 0 && taken)) as u8;
        (direction << 1) | strength
    }
}

/// A mask of the low `bits` bits.
const fn low_bits(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 }
}

impl DirectionPredictor for BhtPredictor {
    type History = BhtHistory;

    /// Predicts taken when the counter's low bit is set (`BHTResp.taken`).
    fn lookup(&self, pc: u64, _target: u64) -> (bool, BhtHistory) {
        let value = self.table[self.index(pc, self.history)];
        let history = BhtHistory { history: self.history, value, class: BranchClass::Conditional };
        (value & 1 == 1, history)
    }

    fn unconditional(&self, _pc: u64, _jump: Jump) -> BhtHistory {
        BhtHistory { history: self.history, value: 0, class: BranchClass::Unconditional }
    }

    /// Advances the history with a predicted branch's direction
    /// (`advanceHistory`); a jump leaves it alone.
    fn update_histories(&mut self, _pc: u64, taken: bool, history: &BhtHistory) {
        if history.class == BranchClass::Conditional {
            self.history = self.shifted(history.history, taken);
        }
    }

    fn squash(&mut self, history: &BhtHistory) {
        self.history = history.history;
    }

    /// Rebuilds the history from a mispredicted branch's fetch-time history
    /// and its real direction (`updateHistory`), or resets it to a
    /// mispredicted jump's (`resetHistory`).
    fn correct(&mut self, _pc: u64, taken: bool, history: &BhtHistory) {
        self.history = match history.class {
            BranchClass::Conditional => self.shifted(history.history, taken),
            BranchClass::Unconditional => history.history,
        };
    }

    /// Trains the counter the prediction read, from the value it read.
    fn commit(&mut self, pc: u64, retired: Retired, history: &BhtHistory) {
        if retired.class != BranchClass::Conditional {
            return;
        }
        let index = self.index(pc, history.history);
        self.table[index] = self.trained(history.value, retired.taken);
    }
}
