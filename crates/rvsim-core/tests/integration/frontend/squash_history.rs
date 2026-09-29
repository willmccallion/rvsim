//! A squash that is not a misprediction undoes what the squashed
//! instructions did to the predictor.
//!
//! A function whose body writes a CSR squashes its own return once fetch
//! has run past it; the refetched return must pop the same address the
//! squashed one did.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const RA: u32 = 1;
const MSCRATCH: u32 = 0x340;
const PROGRAM_BASE: u64 = 0x8000_0000;
const CALLS: usize = 8;
/// The function, after the calls and the final `j .`.
const FUNCTION: i32 = 4 * (CALLS as i32 + 1);

/// `CALLS` calls to a function that writes `mscratch` and returns.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> =
        (0..CALLS).map(|n| i().jal(RA, FUNCTION - 4 * n as i32).build()).collect();
    program.push(i().jal(0, 0).build());
    program.push(i().csrrw(0, MSCRATCH, 0).build());
    program.push(i().jalr(0, RA, 0).build());
    program
}

fn mispredicts(backend: BackendKind) -> f64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(800);

    let retired = ctx.sim.state.harts[0].instructions_retired;
    assert!(retired > 3 * CALLS as u64, "{backend:?}: the calls ran, {retired} retired");
    let path = ctx.sim.state.cores[0].units.stat_paths.bp.spec_mispredicts;
    ctx.sim.state.stats.get(path).unwrap_or(0.0)
}

#[test]
fn a_return_refetched_after_a_csr_squash_pops_the_right_address() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        assert_eq!(mispredicts(backend), 0.0, "{backend:?}");
    }
}
