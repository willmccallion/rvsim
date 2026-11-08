//! A hart outside WFI that retires nothing for the stall limit stops the
//! run with `SimError::Stalled`; WFI and a limit of `None` never do.

use crate::common::{SimError, VirtAddr};
use crate::config::{Config, Console};
use crate::isa::csr::MIE_MTIE;
use crate::isa::encoding::privileged::WFI;
use crate::system::simulator::StopAt;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::X0;

const PROGRAM_BASE: u64 = 0x8000_0000;
/// Fewer cycles than the first instruction takes to fetch from cold caches.
const SHORTER_THAN_A_COLD_FETCH: u64 = 10;

fn system(program: &[u32]) -> TestContext {
    let mut config = Config::minimal();
    config.system.console = Console::Quiet;
    TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program)
}

fn spin() -> Vec<u32> {
    vec![InstructionBuilder::new().jal(X0, 0).build()]
}

fn run_for(ctx: &mut TestContext, cycles: u64) -> Result<(), SimError> {
    ctx.sim.run_to(&StopAt { cycles: Some(cycles), ..StopAt::default() }).map(|_| ())
}

#[test]
fn a_hart_that_retires_nothing_for_the_limit_stops_the_run() {
    let mut ctx = system(&spin());
    ctx.sim.set_stall_limit(Some(SHORTER_THAN_A_COLD_FETCH));

    let stopped = run_for(&mut ctx, 1_000);

    let Err(SimError::Stalled { hart, pc, cycles }) = stopped else {
        panic!("expected a stall, got {stopped:?}");
    };
    assert_eq!((hart, pc, cycles), (0, VirtAddr::new(PROGRAM_BASE), SHORTER_THAN_A_COLD_FETCH));
}

#[test]
fn no_limit_lets_a_slow_hart_run() {
    let mut ctx = system(&spin());
    ctx.sim.set_stall_limit(None);

    assert!(run_for(&mut ctx, 1_000).is_ok());
}

#[test]
fn a_hart_retiring_in_a_loop_never_stalls() {
    let mut ctx = system(&spin());
    run_for(&mut ctx, 2_000).expect("the first fetch completes");
    ctx.sim.set_stall_limit(Some(SHORTER_THAN_A_COLD_FETCH));

    assert!(run_for(&mut ctx, 10_000).is_ok());
}

#[test]
fn a_hart_waiting_in_wfi_never_stalls() {
    let mut ctx = system(&[WFI, InstructionBuilder::new().jal(X0, 0).build()]);
    // A timer interrupt enabled but never due, so WFI waits for good.
    ctx.sim.state.harts[0].csrs.mie = MIE_MTIE;
    run_for(&mut ctx, 2_000).expect("the hart reaches WFI");
    ctx.sim.set_stall_limit(Some(SHORTER_THAN_A_COLD_FETCH));

    assert!(run_for(&mut ctx, 10_000).is_ok());
    assert!(ctx.sim.state.harts[0].wfi_waiting);
}
