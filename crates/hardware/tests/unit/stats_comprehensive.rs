//! Comprehensive tests for simulation statistics.

use rvsim_core::stats::SimStats;

#[test]
fn test_stats_default() {
    let stats = SimStats::default();
    assert_eq!(stats.cycles, 0);
    assert_eq!(stats.instructions_retired, 0);
    assert_eq!(stats.inst_load, 0);
    assert_eq!(stats.inst_store, 0);
    assert_eq!(stats.inst_branch, 0);
    assert_eq!(stats.inst_alu, 0);
    assert_eq!(stats.inst_system, 0);
}

#[test]
fn test_stats_clone() {
    let mut stats = SimStats::default();
    stats.cycles = 100;
    stats.instructions_retired = 50;

    let cloned = stats.clone();
    assert_eq!(cloned.cycles, 100);
    assert_eq!(cloned.instructions_retired, 50);
}

#[test]
fn test_stats_print_all_sections() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.instructions_retired = 500;
    stats.inst_alu = 200;
    stats.inst_load = 150;
    stats.inst_store = 100;
    stats.inst_branch = 50;
    stats.branch_predictions = 40;
    stats.branch_mispredictions = 10;
    stats.icache_hits = 450;
    stats.icache_misses = 50;
    stats.dcache_hits = 200;
    stats.dcache_misses = 50;

    // Should not panic
    stats.print();
}

#[test]
fn test_stats_print_summary_section() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.instructions_retired = 500;

    stats.print_sections(&[String::from("summary")]);
}

#[test]
fn test_stats_print_core_section() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.cycles_user = 400;
    stats.cycles_kernel = 300;
    stats.cycles_machine = 300;
    stats.stalls_mem = 50;
    stats.stalls_control = 30;
    stats.stalls_data = 20;

    stats.print_sections(&[String::from("core")]);
}

#[test]
fn test_stats_print_instruction_mix_section() {
    let mut stats = SimStats::default();
    stats.instructions_retired = 1000;
    stats.inst_alu = 400;
    stats.inst_load = 250;
    stats.inst_store = 150;
    stats.inst_branch = 100;
    stats.inst_system = 50;
    stats.inst_fp_arith = 50;

    stats.print_sections(&[String::from("instruction_mix")]);
}

#[test]
fn test_stats_print_branch_section() {
    let mut stats = SimStats::default();
    stats.branch_predictions = 900;
    stats.branch_mispredictions = 100;

    stats.print_sections(&[String::from("branch")]);
}

#[test]
fn test_stats_print_memory_section() {
    let mut stats = SimStats::default();
    stats.icache_hits = 8000;
    stats.icache_misses = 2000;
    stats.dcache_hits = 6000;
    stats.dcache_misses = 4000;
    stats.l2_hits = 3000;
    stats.l2_misses = 1000;
    stats.l3_hits = 800;
    stats.l3_misses = 200;

    stats.print_sections(&[String::from("memory")]);
}

#[test]
fn test_stats_print_multiple_sections() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.instructions_retired = 500;
    stats.branch_predictions = 40;
    stats.branch_mispredictions = 10;

    stats.print_sections(&[String::from("summary"), String::from("branch")]);
}

#[test]
fn test_stats_zero_cycles_division_safe() {
    let stats = SimStats::default();
    // Should not panic with zero cycles
    stats.print();
}

#[test]
fn test_stats_zero_instructions_division_safe() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    // instructions_retired is 0
    stats.print();
}

#[test]
fn test_stats_zero_branch_predictions_safe() {
    let stats = SimStats::default();
    // branch_predictions and branch_mispredictions are both 0
    stats.print_sections(&[String::from("branch")]);
}

#[test]
fn test_stats_zero_cache_accesses_safe() {
    let stats = SimStats::default();
    // All cache hits and misses are 0
    stats.print_sections(&[String::from("memory")]);
}

#[test]
fn test_stats_high_values() {
    let mut stats = SimStats::default();
    stats.cycles = u64::MAX / 2;
    stats.instructions_retired = u64::MAX / 4;
    stats.print_sections(&[String::from("summary")]);
}

#[test]
fn test_stats_all_instruction_types() {
    let mut stats = SimStats::default();
    stats.instructions_retired = 1000;
    stats.inst_alu = 100;
    stats.inst_load = 100;
    stats.inst_store = 100;
    stats.inst_branch = 100;
    stats.inst_system = 100;
    stats.inst_fp_load = 100;
    stats.inst_fp_store = 100;
    stats.inst_fp_arith = 100;
    stats.inst_fp_fma = 100;
    stats.inst_fp_div_sqrt = 100;

    stats.print_sections(&[String::from("instruction_mix")]);
}

#[test]
fn test_stats_all_stall_types() {
    let mut stats = SimStats::default();
    stats.cycles = 1000;
    stats.stalls_mem = 300;
    stats.stalls_control = 200;
    stats.stalls_data = 100;

    stats.print_sections(&[String::from("core")]);
}

#[test]
fn test_stats_all_privilege_modes() {
    let mut stats = SimStats::default();
    stats.cycles = 900;
    stats.cycles_user = 300;
    stats.cycles_kernel = 300;
    stats.cycles_machine = 300;

    stats.print_sections(&[String::from("core")]);
}

#[test]
fn test_stats_perfect_branch_prediction() {
    let mut stats = SimStats::default();
    stats.branch_predictions = 1000;
    stats.branch_mispredictions = 0;

    stats.print_sections(&[String::from("branch")]);
}

#[test]
fn test_stats_worst_branch_prediction() {
    let mut stats = SimStats::default();
    stats.branch_predictions = 0;
    stats.branch_mispredictions = 1000;

    stats.print_sections(&[String::from("branch")]);
}

#[test]
fn test_stats_perfect_cache_hits() {
    let mut stats = SimStats::default();
    stats.icache_hits = 10000;
    stats.icache_misses = 0;
    stats.dcache_hits = 10000;
    stats.dcache_misses = 0;
    stats.l2_hits = 0; // No L2 access if L1 always hits
    stats.l2_misses = 0;

    stats.print_sections(&[String::from("memory")]);
}

#[test]
fn test_stats_all_cache_misses() {
    let mut stats = SimStats::default();
    stats.icache_hits = 0;
    stats.icache_misses = 1000;
    stats.dcache_hits = 0;
    stats.dcache_misses = 1000;
    stats.l2_hits = 0;
    stats.l2_misses = 1000;
    stats.l3_hits = 0;
    stats.l3_misses = 1000;

    stats.print_sections(&[String::from("memory")]);
}

#[test]
fn test_stats_empty_sections_list() {
    let mut stats = SimStats::default();
    stats.cycles = 100;
    stats.instructions_retired = 50;

    // Empty list means print all sections
    stats.print_sections(&[]);
}

#[test]
fn test_stats_invalid_section_name() {
    let mut stats = SimStats::default();
    stats.cycles = 100;

    // Should not panic, just won't match any section
    stats.print_sections(&[String::from("invalid_section")]);
}

#[test]
fn test_stats_sections_constant() {
    use rvsim_core::stats::STATS_SECTIONS;

    assert!(STATS_SECTIONS.contains(&"summary"));
    assert!(STATS_SECTIONS.contains(&"core"));
    assert!(STATS_SECTIONS.contains(&"instruction_mix"));
    assert!(STATS_SECTIONS.contains(&"branch"));
    assert!(STATS_SECTIONS.contains(&"memory"));
}

#[test]
fn test_stats_traps_counter() {
    let mut stats = SimStats::default();
    stats.traps_taken = 42;
    assert_eq!(stats.traps_taken, 42);
}

#[test]
fn test_stats_fp_counters() {
    let mut stats = SimStats::default();
    stats.inst_fp_load = 10;
    stats.inst_fp_store = 20;
    stats.inst_fp_arith = 30;
    stats.inst_fp_fma = 40;
    stats.inst_fp_div_sqrt = 50;

    assert_eq!(stats.inst_fp_load, 10);
    assert_eq!(stats.inst_fp_store, 20);
    assert_eq!(stats.inst_fp_arith, 30);
    assert_eq!(stats.inst_fp_fma, 40);
    assert_eq!(stats.inst_fp_div_sqrt, 50);
}

#[test]
fn test_stats_realistic_workload() {
    let mut stats = SimStats::default();

    // Simulate a realistic workload
    stats.cycles = 10_000_000;
    stats.instructions_retired = 8_000_000;

    stats.inst_alu = 4_000_000;
    stats.inst_load = 2_000_000;
    stats.inst_store = 1_000_000;
    stats.inst_branch = 800_000;
    stats.inst_system = 200_000;

    stats.branch_predictions = 700_000;
    stats.branch_mispredictions = 100_000;

    stats.icache_hits = 7_500_000;
    stats.icache_misses = 500_000;
    stats.dcache_hits = 2_700_000;
    stats.dcache_misses = 300_000;
    stats.l2_hits = 700_000;
    stats.l2_misses = 100_000;

    stats.stalls_mem = 500_000;
    stats.stalls_control = 300_000;
    stats.stalls_data = 200_000;

    stats.cycles_user = 7_000_000;
    stats.cycles_kernel = 2_000_000;
    stats.cycles_machine = 1_000_000;

    // Should print complete, realistic statistics
    stats.print();
}
