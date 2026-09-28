//! A machine-mode read-modify-write of `mip` while the PLIC holds SEIP
//! high must not turn the line into the software SEIP bit: only that bit
//! takes part in a CSRRS/CSRRC, so SEIP drops with the line.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr::{MIP, MIP_SEIP, MIP_STIP};

const PROGRAM_BASE: u64 = 0x8000_0000;
const PLIC_BASE: u64 = 0x0c00_0000;
const UART_IER: u64 = 0x1000_0001;
const UART_SOURCE: u64 = 10;
const HART0_S_CONTEXT: u64 = 1;
const T0: u32 = 5;
const T1: u32 = 6;

/// Waits for SEIP to show in `mip`, clears STIP with a CSRRC, then spins.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, MIP_STIP as i32).build(),
        i().csrrs(T1, MIP.as_u32(), 0).build(),
        i().andi(T1, T1, MIP_SEIP as i32).build(),
        i().beq(T1, 0, -8).build(),
        i().csrrc(0, MIP.as_u32(), T0).build(),
        i().jal(0, 0).build(),
    ]
}

#[test]
fn clearing_another_mip_bit_while_seip_is_high_does_not_latch_seip() {
    let mut config = Config::default();
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    // The UART's transmit-empty interrupt, enabled for hart 0's S context, holds SEIP high.
    ctx.sim.probe_mem_store(PhysAddr::new(PLIC_BASE + UART_SOURCE * 4), 1, 4);
    ctx.sim.probe_mem_store(
        PhysAddr::new(PLIC_BASE + 0x2000 + HART0_S_CONTEXT * 0x80),
        1 << UART_SOURCE,
        4,
    );
    ctx.sim.probe_mem_store(PhysAddr::new(UART_IER), 0b10, 1);

    ctx.run(800);
    assert_ne!(ctx.sim.state.harts[0].csrs.mip & MIP_SEIP, 0, "the line is high");
    assert_eq!(ctx.sim.state.harts[0].pc, PROGRAM_BASE + 20, "the CSRRC ran and the spin began");

    ctx.sim.probe_mem_store(PhysAddr::new(UART_IER), 0, 1);
    ctx.run(20);

    assert_eq!(ctx.sim.state.harts[0].csrs.mip & MIP_SEIP, 0, "SEIP follows the line");
}
