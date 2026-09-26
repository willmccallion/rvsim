//! CLINT and PLIC lines reach the hart they belong to and no other.

use rvsim_core::Simulator;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr::{MIP_MEIP, MIP_MSIP, MIP_MTIP};

const CLINT_BASE: u64 = 0x0200_0000;
const PLIC_BASE: u64 = 0x0c00_0000;

/// Two harts spinning on a `jal 0` at the reset address, so the cycle loop keeps running.
fn two_hart_sim() -> Simulator {
    let mut config = Config::default();
    config.system.hart_count = 2;
    config.system.uart_quiet = true;
    let mut sim = Simulator::build(&config, "");
    sim.probe_mem_store(PhysAddr::new(config.system.ram_base), 0x0000_006F, 4);
    sim
}

#[test]
fn msip_write_raises_software_interrupt_on_that_hart_only() {
    let mut sim = two_hart_sim();
    sim.probe_mem_store(PhysAddr::new(CLINT_BASE + 4), 1, 4);

    sim.tick().unwrap();

    assert_eq!(sim.state.harts[0].csrs.mip & MIP_MSIP, 0);
    assert_ne!(sim.state.harts[1].csrs.mip & MIP_MSIP, 0);
}

#[test]
fn mtimecmp_of_second_hart_raises_its_timer_line_only() {
    let mut sim = two_hart_sim();
    sim.probe_mem_store(PhysAddr::new(CLINT_BASE + 0x4008), 0, 8);

    sim.tick().unwrap();

    assert_eq!(sim.state.harts[0].csrs.mip & MIP_MTIP, 0);
    assert_ne!(sim.state.harts[1].csrs.mip & MIP_MTIP, 0);
}

#[test]
fn plic_context_of_second_hart_drives_its_external_line_only() {
    let mut sim = two_hart_sim();
    // UART is PLIC source 10; give it a priority and enable it for hart 1's
    // M-mode context (context 2) only, then make the UART raise its line by
    // enabling its transmit-empty interrupt (IER bit 1) with an empty FIFO.
    sim.probe_mem_store(PhysAddr::new(PLIC_BASE + 10 * 4), 1, 4);
    sim.probe_mem_store(PhysAddr::new(PLIC_BASE + 0x2000 + 2 * 0x80), 1 << 10, 4);
    sim.probe_mem_store(PhysAddr::new(0x1000_0000 + 1), 0b10, 1);

    // The UART raises its line 225 ns (540 cycles) on, the PLIC 3 cycles later.
    for _ in 0..560 {
        sim.tick().unwrap();
    }

    assert_eq!(sim.state.harts[0].csrs.mip & MIP_MEIP, 0);
    assert_ne!(sim.state.harts[1].csrs.mip & MIP_MEIP, 0);
}
