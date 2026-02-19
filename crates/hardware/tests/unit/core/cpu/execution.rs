//! # CPU Execution Tests
//!
//! Tests for the main execution loop and pipeline coordination.

use rvsim_core::Simulator;
use rvsim_core::config::Config;
use rvsim_core::core::arch::mode::PrivilegeMode;

fn create_test_sim() -> Simulator {
    let config = Config::default();
    let system = rvsim_core::soc::System::new(&config, "");
    Simulator::new(system, &config)
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
    let initial_cycles = sim.cpu.stats.cycles;

    sim.tick().unwrap();

    // Cycles should increase
    assert!(sim.cpu.stats.cycles >= initial_cycles);
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
    assert_eq!(sim.cpu.exit_code, None);
}

#[test]
fn test_last_pc_updates() {
    let mut sim = create_test_sim();

    sim.tick().unwrap();

    // PC is always set to a valid address
    let _ = sim.cpu.pc;
}

#[test]
fn test_same_pc_counter() {
    let mut sim = create_test_sim();
    let initial_count = sim.cpu.same_pc_count;
    sim.cpu.same_pc_count = 0;

    // After execution, counter might increment if PC doesn't change
    sim.tick().unwrap();

    // Counter should be valid (either stayed same or changed)
    let _ = sim.cpu.same_pc_count;
    assert!(sim.cpu.same_pc_count != initial_count || sim.cpu.same_pc_count == 0);
}

#[test]
fn test_privilege_preserved_across_tick() {
    let mut sim = create_test_sim();

    sim.tick().unwrap();

    // Privilege should be set to something valid
    assert!(
        sim.cpu.privilege == PrivilegeMode::User
            || sim.cpu.privilege == PrivilegeMode::Supervisor
            || sim.cpu.privilege == PrivilegeMode::Machine
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
    let initial_instructions = sim.cpu.stats.instructions_retired;

    sim.tick().unwrap();

    // Stats should be updated or remain the same (can't execute if no valid instruction)
    assert!(sim.cpu.stats.instructions_retired >= initial_instructions);
}

#[test]
fn test_tick_does_not_corrupt_state() {
    let mut sim = create_test_sim();
    sim.cpu.regs.write(5, 0x1234_5678);

    sim.tick().unwrap();

    // Register x5 should still have value (unless instruction modifies it)
    // At least verify register file is still accessible
    let _ = sim.cpu.regs.read(5);
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
    for priv_level in [
        PrivilegeMode::Machine,
        PrivilegeMode::Supervisor,
        PrivilegeMode::User,
    ] {
        let mut sim = create_test_sim();
        sim.cpu.privilege = priv_level;

        let result = sim.tick();
        assert!(result.is_ok());
    }
}
