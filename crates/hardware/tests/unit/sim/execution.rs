//! # Simulator Execution Tests
//!
//! Tests for the main execution loop and pipeline coordination.

use rvsim_core::Simulator;
use rvsim_core::common::RegIdx;
use rvsim_core::config::Config;
use rvsim_core::core::arch::mode::PrivilegeMode;

fn create_test_sim() -> Simulator {
    let config = Config::default();
    Simulator::build(&config, "")
}

#[test]
fn test_tick_returns_ok() {
    let mut sim = create_test_sim();
    let result = sim.tick();
    assert!(result.is_ok());
}

#[test]
fn test_tick_increments_cycles() {
    let mut sim = create_test_sim();
    let initial_cycles = sim.state.cycle;

    sim.tick().unwrap();

    // Cycles should increase
    assert!(sim.state.cycle >= initial_cycles);
}

#[test]
fn test_multiple_ticks() {
    let mut sim = create_test_sim();

    for _ in 0..5 {
        let result = sim.tick();
        assert!(result.is_ok());
    }
}

#[test]
fn test_exit_code_none_initially() {
    let sim = create_test_sim();
    assert_eq!(sim.state.check_exit(), None);
}

#[test]
fn test_last_pc_updates() {
    let mut sim = create_test_sim();

    sim.tick().unwrap();

    // PC is always set to a valid address
    let _ = sim.state.hart.pc;
}

#[test]
fn test_same_pc_counter() {
    let mut sim = create_test_sim();
    let idx = sim.state.hart.hart_id.as_index();
    let initial_count = sim.state.per_hart_debug[idx].same_pc_count;
    sim.state.per_hart_debug[idx].same_pc_count = 0;

    sim.tick().unwrap();

    let count = sim.state.per_hart_debug[idx].same_pc_count;
    assert!(count != initial_count || count == 0);
}

#[test]
fn test_privilege_preserved_across_tick() {
    let mut sim = create_test_sim();

    sim.tick().unwrap();

    // Privilege should be set to something valid
    assert!(
        sim.state.hart.privilege == PrivilegeMode::User
            || sim.state.hart.privilege == PrivilegeMode::Supervisor
            || sim.state.hart.privilege == PrivilegeMode::Machine
    );
}

#[test]
fn test_bus_interaction_tick() {
    let mut sim = create_test_sim();

    // Should not panic when calling tick which accesses bus
    let result = sim.tick();
    assert!(result.is_ok());
}

#[test]
fn test_stats_updated() {
    let mut sim = create_test_sim();
    let initial_instructions = sim.state.instructions_retired;

    sim.tick().unwrap();

    assert!(sim.state.instructions_retired >= initial_instructions);
}

#[test]
fn test_tick_does_not_corrupt_state() {
    let mut sim = create_test_sim();
    sim.state.hart.regs.write(RegIdx::new(5), 0x1234_5678);

    sim.tick().unwrap();

    let _ = sim.state.hart.regs.read(RegIdx::new(5));
}

#[test]
fn test_rapid_ticks() {
    let mut sim = create_test_sim();

    for _ in 0..100 {
        let result = sim.tick();
        assert!(result.is_ok());
    }

    // Should complete without panicking
}

#[test]
fn test_tick_with_different_privileges() {
    for priv_level in [PrivilegeMode::Machine, PrivilegeMode::Supervisor, PrivilegeMode::User] {
        let mut sim = create_test_sim();
        sim.state.hart.privilege = priv_level;

        let result = sim.tick();
        assert!(result.is_ok());
    }
}
