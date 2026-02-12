//! SimStats unit tests.
//!
//! Verifies default initialization, field mutation, and derived metric
//! computation for the simulation statistics structure.

use riscv_core::stats::SimStats;

#[test]
fn default_stats_all_zero() {
    let stats = SimStats::default();
    assert_eq!(stats.cycles, 0);
    assert_eq!(stats.instructions_retired, 0);
    assert_eq!(stats.inst_load, 0);
    assert_eq!(stats.inst_store, 0);
    assert_eq!(stats.inst_branch, 0);
    assert_eq!(stats.inst_alu, 0);
    assert_eq!(stats.inst_system, 0);
    assert_eq!(stats.inst_fp_load, 0);
    assert_eq!(stats.inst_fp_store, 0);
    assert_eq!(stats.inst_fp_arith, 0);
    assert_eq!(stats.inst_fp_fma, 0);
    assert_eq!(stats.inst_fp_div_sqrt, 0);
    assert_eq!(stats.branch_predictions, 0);
    assert_eq!(stats.branch_mispredictions, 0);
    assert_eq!(stats.stalls_mem, 0);
    assert_eq!(stats.stalls_control, 0);
    assert_eq!(stats.stalls_data, 0);
    assert_eq!(stats.traps_taken, 0);
    assert_eq!(stats.icache_hits, 0);
    assert_eq!(stats.icache_misses, 0);
    assert_eq!(stats.dcache_hits, 0);
    assert_eq!(stats.dcache_misses, 0);
    assert_eq!(stats.l2_hits, 0);
    assert_eq!(stats.l2_misses, 0);
    assert_eq!(stats.l3_hits, 0);
    assert_eq!(stats.l3_misses, 0);
}

#[test]
fn stats_field_mutation() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.instructions_retired = 500;
    stats.inst_alu = 200;
    stats.inst_load = 100;
    stats.inst_store = 50;
    stats.inst_branch = 100;
    stats.inst_system = 50;

    assert_eq!(stats.cycles, 1000);
    assert_eq!(stats.instructions_retired, 500);
    assert_eq!(stats.inst_alu, 200);
    assert_eq!(stats.inst_load, 100);
}

#[test]
fn stats_instruction_mix_sums() {
    let mut stats = SimStats::default();
    stats.inst_alu = 40;
    stats.inst_load = 20;
    stats.inst_store = 15;
    stats.inst_branch = 15;
    stats.inst_system = 10;

    let total =
        stats.inst_alu + stats.inst_load + stats.inst_store + stats.inst_branch + stats.inst_system;
    assert_eq!(total, 100);
}

#[test]
fn stats_branch_prediction_accuracy() {
    let mut stats = SimStats::default();
    stats.branch_predictions = 90;
    stats.branch_mispredictions = 10;

    let total = stats.branch_predictions + stats.branch_mispredictions;
    let accuracy = stats.branch_predictions as f64 / total as f64;
    assert!((accuracy - 0.9).abs() < 1e-10);
}

#[test]
fn stats_cache_hit_rate() {
    let mut stats = SimStats::default();
    stats.icache_hits = 950;
    stats.icache_misses = 50;

    let total = stats.icache_hits + stats.icache_misses;
    let hit_rate = stats.icache_hits as f64 / total as f64;
    assert!((hit_rate - 0.95).abs() < 1e-10);
}

#[test]
fn stats_stall_breakdown() {
    let mut stats = SimStats::default();
    stats.stalls_mem = 100;
    stats.stalls_control = 50;
    stats.stalls_data = 30;
    stats.cycles = 1000;

    let total_stalls = stats.stalls_mem + stats.stalls_control + stats.stalls_data;
    assert_eq!(total_stalls, 180);
    let stall_ratio = total_stalls as f64 / stats.cycles as f64;
    assert!((stall_ratio - 0.18).abs() < 1e-10);
}

#[test]
fn stats_mode_cycle_breakdown() {
    let mut stats = SimStats::default();
    stats.cycles_user = 400;
    stats.cycles_kernel = 500;
    stats.cycles_machine = 100;
    stats.cycles = 1000;

    let total = stats.cycles_user + stats.cycles_kernel + stats.cycles_machine;
    assert_eq!(total, stats.cycles);
}

#[test]
fn stats_clone() {
    let mut stats = SimStats::default();
    stats.cycles = 42;
    stats.instructions_retired = 21;

    let cloned = stats.clone();
    assert_eq!(cloned.cycles, 42);
    assert_eq!(cloned.instructions_retired, 21);
}

#[test]
fn stats_fp_instruction_categories() {
    let mut stats = SimStats::default();
    stats.inst_fp_arith = 50;
    stats.inst_fp_fma = 30;
    stats.inst_fp_div_sqrt = 10;
    stats.inst_fp_load = 5;
    stats.inst_fp_store = 5;

    let total_fp = stats.inst_fp_arith
        + stats.inst_fp_fma
        + stats.inst_fp_div_sqrt
        + stats.inst_fp_load
        + stats.inst_fp_store;
    assert_eq!(total_fp, 100);
}

#[test]
fn stats_sections_constant_available() {
    use riscv_core::stats::STATS_SECTIONS;
    assert!(STATS_SECTIONS.contains(&"summary"));
    assert!(STATS_SECTIONS.contains(&"core"));
    assert!(STATS_SECTIONS.contains(&"instruction_mix"));
    assert!(STATS_SECTIONS.contains(&"branch"));
    assert!(STATS_SECTIONS.contains(&"memory"));
    assert_eq!(STATS_SECTIONS.len(), 5);
}
