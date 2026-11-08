//! What one tick of the simulator does to the state around the pipeline.

use crate::Simulator;
use crate::config::Config;
use crate::isa::privileged::PrivilegeMode;
use crate::tests::support::harness::build_simulator;
use crate::tests::support::regs::T0;

fn minimal_simulator() -> Simulator {
    build_simulator(&Config::minimal())
}

#[test]
fn each_tick_advances_the_cycle_by_one() {
    let mut sim = minimal_simulator();
    let start = sim.state.uncore.kernel.cycle;

    for _ in 0..5 {
        sim.tick().expect("tick");
    }

    assert_eq!(sim.state.uncore.kernel.cycle, start + 5);
}

#[test]
fn a_fresh_simulator_has_no_exit_code() {
    let sim = minimal_simulator();

    assert_eq!(sim.state.uncore.peek_exit(), None);
}

#[test]
fn a_tick_that_retires_nothing_leaves_the_registers_alone() {
    let mut sim = minimal_simulator();
    sim.state.harts[0].regs.write(T0, 0x1234_5678);

    sim.tick().expect("tick");

    assert_eq!(sim.state.instructions_retired(), 0);
    assert_eq!(sim.state.harts[0].regs.read(T0), 0x1234_5678);
}

#[test]
fn a_tick_that_retires_nothing_keeps_the_privilege_mode() {
    for mode in [PrivilegeMode::Machine, PrivilegeMode::Supervisor, PrivilegeMode::User] {
        let mut sim = minimal_simulator();
        sim.state.harts[0].privilege = mode;

        sim.tick().expect("tick");

        assert_eq!(sim.state.harts[0].privilege, mode);
    }
}
