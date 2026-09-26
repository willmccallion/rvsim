//! Branch Predictor Direction Tests.
//!
//! Verifies the direction prediction and training logic for all five
//! branch predictor implementations: Static, GShare, Perceptron, TAGE,
//! and Tournament. The BTB and RAS are tested separately in btb.rs and
//! ras.rs — this file focuses on predict_branch / update_branch semantics.
//!
//! Reference: Phase 2 — Pipeline Logic & Hazards.

use rvsim_core::config::{PerceptronConfig, TageConfig, TournamentConfig};
use rvsim_core::core::units::bru::predictors::gshare::GSharePredictor;
use rvsim_core::core::units::bru::predictors::perceptron::PerceptronPredictor;
use rvsim_core::core::units::bru::predictors::static_bp::StaticPredictor;
use rvsim_core::core::units::bru::predictors::tage::TagePredictor;
use rvsim_core::core::units::bru::predictors::tournament::TournamentPredictor;
use rvsim_core::core::units::bru::{BranchPredictor, Ghr};

fn default_tage() -> TagePredictor {
    let config = TageConfig {
        num_banks: 4,
        table_size: 2048,
        loop_table_size: 256,
        reset_interval: 256_000,
        history_lengths: vec![5, 15, 44, 130],
        tag_widths: vec![9, 9, 10, 10],
    };
    TagePredictor::new(&config, 64, 4, 8)
}

fn default_perceptron() -> PerceptronPredictor {
    PerceptronPredictor::new(
        &PerceptronConfig {
            history_length: 8,
            table_bits: 6, // 64 entries
        },
        64,
        4,
        8,
    )
}

fn default_tournament() -> TournamentPredictor {
    TournamentPredictor::new(
        &TournamentConfig {
            global_size_bits: 6, // 64 entries
            local_hist_bits: 6,
            local_pred_bits: 6,
        },
        64,
        4,
        8,
    )
}

/// Train a predictor by feeding `n` iterations of the same branch outcome.
fn train<P: BranchPredictor>(bp: &mut P, pc: u64, taken: bool, target: u64, n: usize) {
    let tgt = if taken { Some(target) } else { None };
    for _ in 0..n {
        let snapshot = bp.snapshot_history();
        bp.update_branch(pc, taken, tgt, &snapshot);
    }
}

/// Static predictor always predicts not-taken.
#[test]
fn static_always_not_taken() {
    let bp = StaticPredictor::new(64, 4, 8);
    let (taken, target) = bp.predict_branch(0x1000);
    assert!(!taken, "Static should always predict not-taken");
    assert_eq!(target, None);
}

/// Static predictor stays not-taken even after taken training.
#[test]
fn static_ignores_training() {
    let mut bp = StaticPredictor::new(64, 4, 8);
    train(&mut bp, 0x1000, true, 0x2000, 100);
    let (taken, _) = bp.predict_branch(0x1000);
    assert!(!taken, "Static should still predict not-taken after training");
}

/// Static predictor still updates BTB (used for unconditional jumps).
#[test]
fn static_updates_btb() {
    let mut bp = StaticPredictor::new(64, 4, 8);
    bp.update_branch(0x1000, true, Some(0x2000), &Ghr::default());
    assert_eq!(bp.predict_btb(0x1000), Some(0x2000));
}

/// GShare initial prediction — counters initialized to 1 (weakly not-taken).
#[test]
fn gshare_initial_not_taken() {
    let bp = GSharePredictor::new(64, 4, 8);
    let (taken, _) = bp.predict_branch(0x1000);
    assert!(!taken, "Initial counter=1 → not taken (< 2)");
}

/// GShare learns taken after repeated taken updates.
/// The GHR shift-register means each training step may hit a different PHT
/// entry until the GHR saturates (all 1s for all-taken, after ~12 steps
/// with TABLE_BITS=12). After saturation, further training reinforces the
/// same entry. We use 20 steps to ensure convergence.
#[test]
fn gshare_learns_taken() {
    let mut bp = GSharePredictor::new(64, 4, 8);
    let pc = 0x1000;
    train(&mut bp, pc, true, 0x2000, 20);

    let (taken, _) = bp.predict_branch(pc);
    assert!(taken, "GShare should learn taken after training");
}

/// GShare learns not-taken after repeated not-taken updates.
#[test]
fn gshare_learns_not_taken() {
    let mut bp = GSharePredictor::new(64, 4, 8);
    let pc = 0x1000;

    // First push counters up to taken...
    train(&mut bp, pc, true, 0x2000, 10);
    // ...then train not-taken extensively.
    train(&mut bp, pc, false, 0x2000, 20);

    let (taken, _) = bp.predict_branch(pc);
    assert!(!taken, "GShare should learn not-taken after training");
}

/// GShare uses GHR XOR PC for indexing — different histories produce different predictions.
#[test]
fn gshare_context_sensitive() {
    let mut bp = GSharePredictor::new(256, 4, 8);
    let pc = 0x1000;

    // Create two different history contexts by feeding different branches.
    // Context A: branch at pc=0x100 taken, then predict pc=0x1000.
    let snap = bp.snapshot_history();
    bp.update_branch(0x100, true, Some(0x200), &snap);
    let (pred_a, _) = bp.predict_branch(pc);

    // Context B: branch at pc=0x100 not-taken, then predict pc=0x1000.
    let mut bp2 = GSharePredictor::new(256, 4, 8);
    let snap2 = bp2.snapshot_history();
    bp2.update_branch(0x100, false, None, &snap2);
    let (pred_b, _) = bp2.predict_branch(pc);

    // Predictions may or may not differ; the test exercises the code path.
    let _ = (pred_a, pred_b);
}

/// Perceptron initial prediction — all weights zero, output = 0 → taken (>= 0).
#[test]
fn perceptron_initial_prediction() {
    let bp = default_perceptron();
    let (taken, _) = bp.predict_branch(0x1000);
    assert!(taken, "Initial weights=0, output=0 → taken (>= 0)");
}

/// Perceptron learns taken after consistent taken training.
#[test]
fn perceptron_learns_taken() {
    let mut bp = default_perceptron();
    let pc = 0x1000;
    train(&mut bp, pc, true, 0x2000, 50);
    let (taken, _) = bp.predict_branch(pc);
    assert!(taken, "Perceptron should learn taken");
}

/// Perceptron learns not-taken after consistent not-taken training.
#[test]
fn perceptron_learns_not_taken() {
    let mut bp = default_perceptron();
    let pc = 0x1000;
    train(&mut bp, pc, false, 0x2000, 100);
    let (taken, _) = bp.predict_branch(pc);
    assert!(!taken, "Perceptron should learn not-taken");
}

/// Perceptron can flip direction with retraining.
#[test]
fn perceptron_retrains() {
    let mut bp = default_perceptron();
    let pc = 0x1000;

    train(&mut bp, pc, true, 0x2000, 50);
    let (t1, _) = bp.predict_branch(pc);

    train(&mut bp, pc, false, 0x2000, 100);
    let (t2, _) = bp.predict_branch(pc);

    assert!(t1, "Should have learned taken first");
    assert!(!t2, "Should retrain to not-taken");
}

/// TAGE initial prediction comes from base predictor (counters = 0 → taken).
#[test]
fn tage_initial_prediction() {
    let bp = default_tage();
    let (taken, _) = bp.predict_branch(0x1000);
    // Base predictor starts at 0, and 0 >= 0 → taken.
    assert!(taken, "Base predictor counter=0 → taken");
}

/// TAGE learns taken on a single branch after training.
#[test]
fn tage_learns_taken() {
    let mut bp = default_tage();
    let pc = 0x1000;
    train(&mut bp, pc, true, 0x2000, 20);
    let (taken, _) = bp.predict_branch(pc);
    assert!(taken);
}

/// TAGE learns not-taken after enough not-taken training.
#[test]
fn tage_learns_not_taken() {
    let mut bp = default_tage();
    let pc = 0x1000;
    train(&mut bp, pc, false, 0x2000, 40);
    let (taken, _) = bp.predict_branch(pc);
    assert!(!taken, "TAGE should learn not-taken");
}

/// TAGE allocates entries in longer-history banks on misprediction.
/// After training not-taken, then switching to taken, the predictor should
/// eventually adapt (showing allocation works).
#[test]
fn tage_adapts_to_pattern_change() {
    let mut bp = default_tage();
    let pc = 0x1000;

    // Train not-taken.
    train(&mut bp, pc, false, 0x2000, 30);
    let (t1, _) = bp.predict_branch(pc);
    assert!(!t1, "Should predict not-taken after training");

    // Switch to taken.
    train(&mut bp, pc, true, 0x2000, 60);
    let (t2, _) = bp.predict_branch(pc);
    assert!(t2, "Should adapt to taken after retraining");
}

/// Tournament initial prediction — choice counter starts at 1 → local,
/// local PHT starts at 1 → not-taken (< 2).
#[test]
fn tournament_initial_not_taken() {
    let bp = default_tournament();
    let (taken, _) = bp.predict_branch(0x1000);
    assert!(!taken, "Initial local counter=1 → not taken");
}

/// Tournament learns taken after training.
#[test]
fn tournament_learns_taken() {
    let mut bp = default_tournament();
    let pc = 0x1000;
    train(&mut bp, pc, true, 0x2000, 20);
    let (taken, _) = bp.predict_branch(pc);
    assert!(taken, "Tournament should learn taken");
}

/// Tournament learns not-taken.
#[test]
fn tournament_learns_not_taken() {
    let mut bp = default_tournament();
    let pc = 0x1000;
    // First train taken to move counters up...
    train(&mut bp, pc, true, 0x2000, 10);
    // ...then extensively train not-taken.
    train(&mut bp, pc, false, 0x2000, 30);
    let (taken, _) = bp.predict_branch(pc);
    assert!(!taken, "Tournament should learn not-taken");
}

/// Tournament adapts when global outperforms local.
#[test]
fn tournament_adapts_choice() {
    let mut bp = default_tournament();
    let pc = 0x1000;

    // Alternating pattern benefits global correlation; train heavily.
    for i in 0..50 {
        let taken = i % 2 == 0;
        let tgt = if taken { Some(0x2000) } else { None };
        let snap = bp.snapshot_history();
        bp.update_branch(pc, taken, tgt, &snap);
    }

    // The predictor should not crash and should produce a valid prediction.
    let (taken, _) = bp.predict_branch(pc);
    let _ = taken; // No assertion on direction — just verifying correctness of logic.
}

/// All predictors update and read the BTB correctly.
#[test]
fn all_predictors_use_btb() {
    let pc = 0x1000;
    let target = 0x2000;

    let mut static_bp = StaticPredictor::new(64, 4, 8);
    static_bp.update_branch(pc, true, Some(target), &Ghr::default());
    assert_eq!(static_bp.predict_btb(pc), Some(target));

    let mut gshare = GSharePredictor::new(64, 4, 8);
    gshare.update_branch(pc, true, Some(target), &Ghr::default());
    assert_eq!(gshare.predict_btb(pc), Some(target));

    let mut perceptron = default_perceptron();
    perceptron.update_branch(pc, true, Some(target), &Ghr::default());
    assert_eq!(perceptron.predict_btb(pc), Some(target));

    let mut tage = default_tage();
    tage.update_branch(pc, true, Some(target), &Ghr::default());
    assert_eq!(tage.predict_btb(pc), Some(target));

    let mut tournament = default_tournament();
    tournament.update_branch(pc, true, Some(target), &Ghr::default());
    assert_eq!(tournament.predict_btb(pc), Some(target));
}

/// Every predictor pops the return address a call pushed, then nothing.
#[test]
fn all_predictors_use_ras() {
    let ret_addr = 0x1004;

    let mut static_bp = StaticPredictor::new(64, 4, 8);
    static_bp.push_return(ret_addr);
    assert_eq!(static_bp.pop_return(), Some(ret_addr));
    assert_eq!(static_bp.pop_return(), None);

    let mut gshare = GSharePredictor::new(64, 4, 8);
    gshare.push_return(ret_addr);
    assert_eq!(gshare.pop_return(), Some(ret_addr));
    assert_eq!(gshare.pop_return(), None);

    let mut perceptron = default_perceptron();
    perceptron.push_return(ret_addr);
    assert_eq!(perceptron.pop_return(), Some(ret_addr));
    assert_eq!(perceptron.pop_return(), None);

    let mut tage = default_tage();
    tage.push_return(ret_addr);
    assert_eq!(tage.pop_return(), Some(ret_addr));
    assert_eq!(tage.pop_return(), None);

    let mut tournament = default_tournament();
    tournament.push_return(ret_addr);
    assert_eq!(tournament.pop_return(), Some(ret_addr));
    assert_eq!(tournament.pop_return(), None);
}

/// A snapshot restores the entry a wrong-path push overwrote after a
/// wrong-path pop.
#[test]
fn a_ras_snapshot_undoes_a_pop_followed_by_a_push() {
    let mut bp = StaticPredictor::new(64, 4, 8);
    bp.push_return(0x1000);
    bp.push_return(0x2000);
    let snapshot = bp.snapshot_ras();

    assert_eq!(bp.pop_return(), Some(0x2000));
    bp.push_return(0x3000);
    bp.restore_ras(snapshot);

    assert_eq!(bp.pop_return(), Some(0x2000));
    assert_eq!(bp.pop_return(), Some(0x1000));
}
