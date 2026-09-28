//! Tournament Branch Predictor, after gem5's `TournamentBP` (the Alpha
//! 21264 predictor).
//!
//! A local predictor indexed by per-branch history and a global predictor
//! indexed by global history, with a choice predictor, also indexed by
//! global history, picking between them. Both histories are updated
//! speculatively at prediction and restored from each squashed
//! prediction's record; the counters train at commit on what the
//! prediction read.

use crate::config::TournamentConfig;
use crate::core::units::bru::direction::{DirectionPredictor, Jump, Retired};

/// Instruction address bits below the local history index (gem5's
/// `instShiftAmt`).
const INST_SHIFT: u32 = 2;
/// Largest value of a 2-bit saturating counter.
const COUNTER_MAX: u8 = 3;
/// A counter above this predicts taken (gem5's threshold for 2 bits).
const COUNTER_THRESHOLD: u8 = 1;

/// Tournament Predictor structure.
#[derive(Debug)]
pub struct TournamentPredictor {
    /// Speculative global history, newest outcome in bit 0.
    global_history: u64,
    /// Bits of global history kept: enough to index the larger of the
    /// global and choice tables.
    history_register_mask: u64,
    global_counters: Vec<u8>,
    choice_counters: Vec<u8>,
    /// Speculative local history of each branch, indexed by its address.
    local_history_table: Vec<u64>,
    /// Counters indexed by a branch's local history.
    local_counters: Vec<u8>,
}

/// What a tournament prediction read, gem5's `BPHistory`.
#[derive(Clone, Copy, Debug)]
pub struct TournamentHistory {
    /// The global history before the prediction.
    global_history: u64,
    local_taken: bool,
    global_taken: bool,
    /// The local history the prediction read, for a conditional branch;
    /// jumps neither read nor shift local history.
    local: Option<LocalHistory>,
}

/// A branch's entry in the local history table and the history it held.
#[derive(Clone, Copy, Debug)]
struct LocalHistory {
    table_index: usize,
    history: usize,
}

impl TournamentPredictor {
    /// Creates a Tournament Predictor: global and choice tables of
    /// `2^global_size_bits` counters, a local history table of
    /// `2^local_hist_bits` entries and `2^local_pred_bits` local counters.
    pub fn new(config: &TournamentConfig) -> Self {
        let global_size = 1usize << config.global_size_bits;
        Self {
            global_history: 0,
            history_register_mask: (1u64 << config.global_size_bits) - 1,
            global_counters: vec![0; global_size],
            choice_counters: vec![0; global_size],
            local_history_table: vec![0; 1 << config.local_hist_bits],
            local_counters: vec![0; 1 << config.local_pred_bits],
        }
    }

    /// The table index both global-history tables use.
    const fn global_index(&self, global_history: u64) -> usize {
        (global_history & self.history_register_mask) as usize
    }

    const fn local_history_index(&self, pc: u64) -> usize {
        (pc >> INST_SHIFT) as usize & (self.local_history_table.len() - 1)
    }

    const fn local_counter_index(&self, local_history: u64) -> usize {
        local_history as usize & (self.local_counters.len() - 1)
    }

    const fn shifted(&self, global_history: u64, taken: bool) -> u64 {
        ((global_history << 1) | taken as u64) & self.history_register_mask
    }
}

const fn predicts_taken(counter: u8) -> bool {
    counter > COUNTER_THRESHOLD
}

const fn train(counter: &mut u8, taken: bool) {
    if taken {
        if *counter < COUNTER_MAX {
            *counter += 1;
        }
    } else if *counter > 0 {
        *counter -= 1;
    }
}

impl DirectionPredictor for TournamentPredictor {
    type History = TournamentHistory;

    fn lookup(&self, pc: u64, _target: u64) -> (bool, TournamentHistory) {
        let table_index = self.local_history_index(pc);
        let history = self.local_counter_index(self.local_history_table[table_index]);
        let local_taken = predicts_taken(self.local_counters[history]);
        let global_index = self.global_index(self.global_history);
        let global_taken = predicts_taken(self.global_counters[global_index]);
        let use_global = predicts_taken(self.choice_counters[global_index]);
        let record = TournamentHistory {
            global_history: self.global_history,
            local_taken,
            global_taken,
            local: Some(LocalHistory { table_index, history }),
        };
        (if use_global { global_taken } else { local_taken }, record)
    }

    fn unconditional(&self, _pc: u64, _jump: Jump) -> TournamentHistory {
        TournamentHistory {
            global_history: self.global_history,
            local_taken: true,
            global_taken: true,
            local: None,
        }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, history: &TournamentHistory) {
        self.global_history = self.shifted(self.global_history, taken);
        if let Some(local) = history.local {
            let entry = &mut self.local_history_table[local.table_index];
            *entry = (*entry << 1) | taken as u64;
        }
    }

    fn squash(&mut self, history: &TournamentHistory) {
        self.global_history = history.global_history;
        if let Some(local) = history.local {
            self.local_history_table[local.table_index] = local.history as u64;
        }
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &TournamentHistory) {
        self.global_history = self.shifted(history.global_history, taken);
        if let Some(local) = history.local {
            self.local_history_table[local.table_index] =
                ((local.history as u64) << 1) | taken as u64;
        }
    }

    /// Trains the choice counter toward whichever component was right when
    /// they disagreed, then the global and local counters the prediction
    /// read. A jump trains only its global counter.
    fn commit(&mut self, _pc: u64, retired: Retired, history: &TournamentHistory) {
        let global_index = self.global_index(history.global_history);
        let Some(local) = history.local else {
            train(&mut self.global_counters[global_index], retired.taken);
            return;
        };
        if history.local_taken != history.global_taken {
            train(&mut self.choice_counters[global_index], history.global_taken == retired.taken);
        }
        train(&mut self.global_counters[global_index], retired.taken);
        train(&mut self.local_counters[local.history], retired.taken);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::units::bru::direction::BranchClass;

    fn predictor() -> TournamentPredictor {
        TournamentPredictor::new(&TournamentConfig {
            global_size_bits: 6,
            local_hist_bits: 6,
            local_pred_bits: 6,
        })
    }

    const fn conditional(taken: bool) -> Retired {
        Retired { class: BranchClass::Conditional, taken, indirect_target: None }
    }

    #[test]
    fn squashing_a_branch_restores_its_local_history() {
        let mut bp = predictor();
        let pc = 0x8000_0010;
        let (_, older) = bp.lookup(pc, 0);
        bp.update_histories(pc, true, &older);
        let before = bp.local_history_table[bp.local_history_index(pc)];

        let (_, younger) = bp.lookup(pc, 0);
        bp.update_histories(pc, true, &younger);
        bp.squash(&younger);

        assert_eq!(bp.local_history_table[bp.local_history_index(pc)], before);
    }

    #[test]
    fn correcting_a_branch_shifts_its_real_outcome_into_both_histories() {
        let mut bp = predictor();
        let pc = 0x8000_0010;
        let (predicted, history) = bp.lookup(pc, 0);
        bp.update_histories(pc, predicted, &history);

        bp.correct(pc, !predicted, &history);

        assert_eq!(bp.global_history & 1, u64::from(!predicted));
        assert_eq!(bp.local_history_table[bp.local_history_index(pc)] & 1, u64::from(!predicted));
    }

    #[test]
    fn the_choice_counter_moves_toward_the_component_that_was_right() {
        let mut bp = predictor();
        let pc = 0x8000_0010;
        let record = TournamentHistory {
            global_history: 0,
            local_taken: false,
            global_taken: true,
            local: Some(LocalHistory { table_index: bp.local_history_index(pc), history: 0 }),
        };

        bp.commit(pc, conditional(true), &record);

        assert_eq!(bp.choice_counters[0], 1);
    }

    #[test]
    fn a_jump_trains_the_global_counter_for_its_history_as_taken() {
        let mut bp = predictor();
        let record = bp.unconditional(0x8000_0010, Jump::Direct);

        bp.commit(
            0x8000_0010,
            Retired { class: BranchClass::Unconditional, taken: true, indirect_target: None },
            &record,
        );

        assert_eq!(bp.global_counters[0], 1);
        assert!(bp.local_counters.iter().all(|&counter| counter == 0));
    }
}
