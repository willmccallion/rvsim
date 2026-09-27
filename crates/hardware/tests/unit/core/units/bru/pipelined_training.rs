//! Predictors trained the way the pipeline trains them.
//!
//! Several control instructions are predicted, and shift the speculative
//! histories, before the oldest commits and trains with the record of its
//! own prediction.

use std::collections::VecDeque;

use rvsim_core::common::InstSeq;
use rvsim_core::config::{PerceptronConfig, TageConfig};
use rvsim_core::core::units::bru::predictors::gshare::GSharePredictor;
use rvsim_core::core::units::bru::predictors::perceptron::PerceptronPredictor;
use rvsim_core::core::units::bru::predictors::tage::TagePredictor;
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
/// half of a loop that also holds a jump and an always-taken back-edge,
/// with `IN_FLIGHT` control instructions between prediction and commit.
fn late_mispredictions<P: DirectionPredictor>(direction: P) -> usize {
    let mut unit = BranchPredUnit::new(direction, 64, 4, 8);
    let mut in_flight = VecDeque::new();
    let mut next_seq = 0;
    let mut mispredictions = 0;
    let mut fetch = |unit: &mut BranchPredUnit<P>, pc, inst, taken, target| {
        let seq = InstSeq::new(next_seq);
        next_seq += 1;
        let predicted = unit.predict(seq, pc, inst).is_some();
        if predicted != taken {
            unit.mispredict(seq, taken, target);
        }
        in_flight.push_back(seq);
        if in_flight.len() > IN_FLIGHT
            && let Some(oldest) = in_flight.pop_front()
        {
            unit.commit(oldest);
        }
        predicted
    };
    for i in 0..ITERATIONS {
        let taken = i % 2 == 0;
        let predicted = fetch(&mut unit, PC, ControlInst::Branch { target: TARGET }, taken, TARGET);
        if i >= ITERATIONS / 2 && predicted != taken {
            mispredictions += 1;
        }
        let jump = ControlInst::Jump { target: JUMP_TARGET, link: None };
        let _ = fetch(&mut unit, JUMP_PC, jump, true, JUMP_TARGET);
        let back_edge = ControlInst::Branch { target: LOOP_TARGET };
        let _ = fetch(&mut unit, LOOP_PC, back_edge, true, LOOP_TARGET);
    }
    mispredictions
}

#[test]
fn gshare_learns_an_alternating_branch_while_others_are_in_flight() {
    assert_eq!(late_mispredictions(GSharePredictor::new()), 0);
}

#[test]
fn perceptron_learns_an_alternating_branch_while_others_are_in_flight() {
    let config = PerceptronConfig { history_length: 16, table_bits: 8 };
    assert_eq!(late_mispredictions(PerceptronPredictor::new(&config)), 0);
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
    assert_eq!(late_mispredictions(TagePredictor::new(&config)), 0);
}
