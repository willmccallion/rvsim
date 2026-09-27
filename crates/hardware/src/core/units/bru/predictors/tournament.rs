//! Tournament Branch Predictor.
//!
//! A hybrid predictor that employs a meta-predictor (Choice PHT) to select
//! between a Global predictor (GShare-like) and a Local predictor (PAg/PAp).
//! This allows the predictor to adapt to different types of branch behaviors.

use crate::config::TournamentConfig;
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};

/// Tournament Predictor structure.
#[derive(Debug)]
pub struct TournamentPredictor {
    /// Global history as fetched, which predictions use.
    ghr: u64,

    /// Global Pattern History Table (2-bit counters).
    global_pht: Vec<u8>,
    /// Mask for indexing the global PHT.
    global_mask: usize,

    /// Local History Table storing history patterns per branch.
    local_history_table: Vec<u16>,
    /// Mask for indexing the Local History Table.
    local_hist_mask: usize,

    /// Local Pattern History Table indexed by local history patterns.
    local_pht: Vec<u8>,
    /// Mask for indexing the Local PHT.
    local_pred_mask: usize,

    /// Choice Prediction Table (2-bit counters).
    /// Selects between Local (0,1) and Global (2,3) predictors.
    choice_pht: Vec<u8>,
}

/// The global history a tournament prediction was made with.
#[derive(Clone, Copy, Debug)]
pub struct TournamentHistory {
    ghr: u64,
}

impl TournamentPredictor {
    /// Creates a new Tournament Predictor based on the provided configuration.
    pub fn new(config: &TournamentConfig) -> Self {
        let global_size = 1 << config.global_size_bits;
        let local_hist_size = 1 << config.local_hist_bits;
        let local_pred_size = 1 << config.local_pred_bits;

        Self {
            ghr: 0,

            global_pht: vec![1; global_size],
            global_mask: global_size - 1,

            local_history_table: vec![0; local_hist_size],
            local_hist_mask: local_hist_size - 1,

            local_pht: vec![1; local_pred_size],
            local_pred_mask: local_pred_size - 1,

            choice_pht: vec![1; global_size],
        }
    }

    const fn global_index(&self, pc: u64, ghr: u64) -> usize {
        ((ghr ^ pc) as usize) & self.global_mask
    }

    const fn shifted(&self, ghr: u64, taken: bool) -> u64 {
        ((ghr << 1) | (taken as u64)) & (self.global_mask as u64)
    }

    /// Retrieves the prediction from the Global component.
    fn get_global_prediction(&self, idx: usize) -> bool {
        self.global_pht[idx] >= 2
    }

    /// Retrieves the prediction from the Local component.
    fn get_local_prediction(&self, pc: u64) -> bool {
        let lh_idx = (pc as usize) & self.local_hist_mask;
        let pattern = self.local_history_table[lh_idx];
        let pred_idx = (pattern as usize) & self.local_pred_mask;
        self.local_pht[pred_idx] >= 2
    }
}

impl DirectionPredictor for TournamentPredictor {
    type History = TournamentHistory;

    /// Queries both Global and Local predictors and uses the Choice PHT to
    /// decide which prediction to use.
    fn lookup(&self, pc: u64) -> (bool, TournamentHistory) {
        let g_idx = self.global_index(pc, self.ghr);
        let global_taken = self.get_global_prediction(g_idx);
        let local_taken = self.get_local_prediction(pc);
        let use_global = self.choice_pht[g_idx] >= 2;
        let taken = if use_global { global_taken } else { local_taken };
        (taken, TournamentHistory { ghr: self.ghr })
    }

    fn unconditional(&self, _pc: u64) -> TournamentHistory {
        TournamentHistory { ghr: self.ghr }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, _history: &TournamentHistory) {
        self.ghr = self.shifted(self.ghr, taken);
    }

    fn squash(&mut self, history: &TournamentHistory) {
        self.ghr = history.ghr;
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &TournamentHistory) {
        self.ghr = self.shifted(history.ghr, taken);
    }

    /// Updates the Choice PHT based on which predictor was correct, then
    /// the Global and Local tables and the local history. The global
    /// entries are the ones the prediction read, found from the history
    /// the branch was predicted with.
    fn commit(&mut self, pc: u64, retired: Retired, history: &TournamentHistory) {
        if retired.class != BranchClass::Conditional {
            return;
        }
        let taken = retired.taken;
        let g_idx = self.global_index(pc, history.ghr);

        let global_pred = self.get_global_prediction(g_idx);
        let local_pred = self.get_local_prediction(pc);

        let global_correct = global_pred == taken;
        let local_correct = local_pred == taken;

        if global_correct != local_correct {
            let choice = &mut self.choice_pht[g_idx];
            if global_correct {
                if *choice < 3 {
                    *choice += 1;
                }
            } else if *choice > 0 {
                *choice -= 1;
            }
        }

        let g_cnt = &mut self.global_pht[g_idx];
        if taken {
            if *g_cnt < 3 {
                *g_cnt += 1;
            }
        } else if *g_cnt > 0 {
            *g_cnt -= 1;
        }

        let lh_idx = (pc as usize) & self.local_hist_mask;
        let pattern = self.local_history_table[lh_idx];
        let pred_idx = (pattern as usize) & self.local_pred_mask;

        let l_cnt = &mut self.local_pht[pred_idx];
        if taken {
            if *l_cnt < 3 {
                *l_cnt += 1;
            }
        } else if *l_cnt > 0 {
            *l_cnt -= 1;
        }

        self.local_history_table[lh_idx] =
            ((pattern << 1) | (taken as u16)) & (self.local_pred_mask as u16);
    }
}
