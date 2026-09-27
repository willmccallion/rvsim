//! Predictors trained the way the pipeline trains them.
//!
//! Several control instructions are predicted, and shift the speculative
//! histories, before the oldest commits and trains with the record of its
//! own prediction.

use std::collections::VecDeque;

use rvsim_core::common::InstSeq;
use rvsim_core::config::{PerceptronConfig, TageConfig, TournamentConfig};
use rvsim_core::core::units::bru::predictors::gshare::GSharePredictor;
use rvsim_core::core::units::bru::predictors::perceptron::PerceptronPredictor;
use rvsim_core::core::units::bru::predictors::tage::TagePredictor;
use rvsim_core::core::units::bru::predictors::tournament::TournamentPredictor;
use rvsim_core::core::units::bru::{BranchPredUnit, ControlInst, DirectionPredictor};

const PC: u64 = 0x8000_0734;
const TARGET: u64 = 0x8000_0744;
/// The loop's back-edge, taken every iteration.
const LOOP_PC: u64 = 0x8000_075c;
const LOOP_TARGET: u64 = 0x8000_072c;
/// A jump in the loop body.
const JUMP_PC: u64 = 0x8000_0748;
const JUMP_TARGET: u64 = 0x8000_0750;
const ITERATIONS: usize = 2_000;
/// Control instructions in flight between fetch and commit.
const IN_FLIGHT: usize = 6;

/// Mispredictions of an alternating taken/not-taken branch over the second
/// half of a loop that also holds a jump and an always-taken back-edge.
/// During iteration `i` at most `window(i)` control instructions are in
/// flight between prediction and commit.
fn late_mispredictions<P: DirectionPredictor>(direction: P, window: fn(usize) -> usize) -> usize {
    let mut unit = BranchPredUnit::new(direction, 64, 4, 8);
    let mut in_flight = VecDeque::new();
    let mut next_seq = 0;
    let mut mispredictions = 0;
    let mut fetch = |unit: &mut BranchPredUnit<P>, depth, pc, inst, taken, target| {
        let seq = InstSeq::new(next_seq);
        next_seq += 1;
        let predicted = unit.predict(seq, pc, inst).is_some();
        if predicted != taken {
            unit.mispredict(seq, taken, target);
        }
        in_flight.push_back(seq);
        while in_flight.len() > depth
            && let Some(oldest) = in_flight.pop_front()
        {
            unit.commit(oldest);
        }
        predicted
    };
    for i in 0..ITERATIONS {
        let depth = window(i);
        let taken = i % 2 == 0;
        let branch = ControlInst::Branch { target: TARGET };
        let predicted = fetch(&mut unit, depth, PC, branch, taken, TARGET);
        if i >= ITERATIONS / 2 && predicted != taken {
            mispredictions += 1;
        }
        let jump = ControlInst::Jump { target: JUMP_TARGET, link: None };
        let _ = fetch(&mut unit, depth, JUMP_PC, jump, true, JUMP_TARGET);
        let back_edge = ControlInst::Branch { target: LOOP_TARGET };
        let _ = fetch(&mut unit, depth, LOOP_PC, back_edge, true, LOOP_TARGET);
    }
    mispredictions
}

const fn steady_window(_iteration: usize) -> usize {
    IN_FLIGHT
}

/// A window that fills and drains the way a pipeline's does, from empty
/// to ten control instructions.
const fn irregular_window(iteration: usize) -> usize {
    (iteration * 7) % 11
}

#[test]
fn gshare_learns_an_alternating_branch_while_others_are_in_flight() {
    assert_eq!(late_mispredictions(GSharePredictor::new(), steady_window), 0);
}

#[test]
fn perceptron_learns_an_alternating_branch_while_others_are_in_flight() {
    let config = PerceptronConfig { history_length: 16, table_bits: 8 };
    assert_eq!(late_mispredictions(PerceptronPredictor::new(&config), steady_window), 0);
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
    assert_eq!(late_mispredictions(TagePredictor::new(&config), steady_window), 0);
}

#[test]
fn tournament_learns_an_alternating_branch_while_others_are_in_flight() {
    let config =
        TournamentConfig { global_size_bits: 13, local_hist_bits: 11, local_pred_bits: 11 };
    assert_eq!(late_mispredictions(TournamentPredictor::new(&config), steady_window), 0);
}

#[test]
fn tournament_learns_an_alternating_branch_as_the_window_fills_and_drains() {
    let config =
        TournamentConfig { global_size_bits: 13, local_hist_bits: 11, local_pred_bits: 11 };
    assert_eq!(late_mispredictions(TournamentPredictor::new(&config), irregular_window), 0);
}
