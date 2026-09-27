//! Predictors trained the way the pipeline trains them.
//!
//! Several branches are predicted, and shift the speculative history, before
//! the oldest commits and trains with the history snapshot taken when it was
//! fetched.

use std::collections::VecDeque;

use rvsim_core::config::{PerceptronConfig, TageConfig};
use rvsim_core::core::units::bru::predictors::gshare::GSharePredictor;
use rvsim_core::core::units::bru::predictors::perceptron::PerceptronPredictor;
use rvsim_core::core::units::bru::predictors::tage::TagePredictor;
use rvsim_core::core::units::bru::{BranchPredictor, Ghr};

const PC: u64 = 0x8000_0734;
const TARGET: u64 = 0x8000_0744;
/// The loop's back-edge, taken every iteration.
const LOOP_PC: u64 = 0x8000_075c;
const LOOP_TARGET: u64 = 0x8000_072c;
const ITERATIONS: usize = 2_000;
/// Branches in flight between fetch and commit.
const IN_FLIGHT: usize = 6;

/// Mispredictions of an alternating taken/not-taken branch over the second
/// half of a loop whose always-taken back-edge follows it, with `IN_FLIGHT`
/// branches between prediction and training.
fn late_mispredictions<P: BranchPredictor>(bp: &mut P) -> usize {
    let mut pending: VecDeque<(u64, bool, u64, Ghr)> = VecDeque::new();
    let mut mispredictions = 0;
    for i in 0..ITERATIONS {
        let branches = [(PC, i % 2 == 0, TARGET), (LOOP_PC, true, LOOP_TARGET)];
        for (pc, taken, target) in branches {
            let snapshot = bp.snapshot_history();
            let (predicted, _) = bp.predict_branch(pc);
            if pc == PC && i >= ITERATIONS / 2 && predicted != taken {
                mispredictions += 1;
            }
            if predicted != taken {
                bp.repair_history(&snapshot);
            }
            bp.speculate(pc, taken);
            pending.push_back((pc, taken, target, snapshot));
            if pending.len() > IN_FLIGHT {
                let (pc, outcome, target, snap) = pending.pop_front().unwrap();
                bp.update_branch(pc, outcome, outcome.then_some(target), &snap);
            }
        }
    }
    mispredictions
}

#[test]
fn gshare_learns_an_alternating_branch_while_others_are_in_flight() {
    assert_eq!(late_mispredictions(&mut GSharePredictor::new(64, 4, 8)), 0);
}

#[test]
fn perceptron_learns_an_alternating_branch_while_others_are_in_flight() {
    let config = PerceptronConfig { history_length: 16, table_bits: 8 };
    assert_eq!(late_mispredictions(&mut PerceptronPredictor::new(&config, 64, 4, 8)), 0);
}

#[test]
fn tage_learns_an_alternating_branch_while_others_are_in_flight() {
    let config = TageConfig {
        num_banks: 4,
        table_size: 2048,
        loop_table_size: 256,
        reset_interval: 256_000,
        history_lengths: vec![5, 15, 44, 130],
        tag_widths: vec![9, 9, 10, 10],
    };
    assert_eq!(late_mispredictions(&mut TagePredictor::new(&config, 64, 4, 8)), 0);
}
