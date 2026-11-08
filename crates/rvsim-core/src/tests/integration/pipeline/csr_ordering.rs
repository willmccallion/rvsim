//! A CSR read must see the CSR write of the instruction before it, on
//! every backend: the kernel's trap entry swaps `tp` with `sscratch` and
//! then reads `sscratch` back, and a stale read leaves `tp` zero.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{T0, T1, T2, X0};
const MSCRATCH: u32 = 0x340;
const PROGRAM_BASE: u64 = 0x8000_0000;

fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, X0, 0x123).build(),
        i().csrrw(X0, MSCRATCH, X0).build(),
        i().csrrw(T0, MSCRATCH, T0).build(),
        i().csrrs(T1, MSCRATCH, X0).build(),
        i().csrrw(T2, MSCRATCH, X0).build(),
        i().jal(X0, 0).build(),
    ]
}

fn swap_then_read_back(backend: BackendKind, width: usize) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    assert_eq!(ctx.get_reg(T0), 0, "{backend:?} w{width}: the swap returns the old value");
    assert_eq!(ctx.get_reg(T1), 0x123, "{backend:?} w{width}: the read sees the swapped-in value");
    assert_eq!(ctx.get_reg(T2), 0x123, "{backend:?} w{width}: the second swap sees it too");
}

#[test]
fn an_inorder_csr_read_sees_the_preceding_write() {
    swap_then_read_back(BackendKind::InOrder, 1);
    swap_then_read_back(BackendKind::InOrder, 4);
}

#[test]
fn an_o3_csr_read_sees_the_preceding_write() {
    swap_then_read_back(BackendKind::OutOfOrder, 1);
    swap_then_read_back(BackendKind::OutOfOrder, 4);
    swap_then_read_back(BackendKind::OutOfOrder, 10);
}

/// O3 serializes after a CSR access the way gem5 does: the instruction
/// behind it waits in rename until the ROB drains, instead of being fetched,
/// squashed and fetched again.
#[test]
fn o3_holds_rename_behind_a_csr_access_instead_of_squashing() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    let paths = &ctx.sim.state.cores[0].units.stat_paths.pipeline;
    let stats = &ctx.sim.state.uncore.kernel.stats;
    assert_eq!(ctx.get_reg(T1), 0x123, "the read sees the swapped-in value");
    assert_eq!(stats.get(paths.flushes_system), Some(0.0), "no CSR access squashed");
    assert!(stats.get(paths.stalls_serialize).unwrap_or(0.0) > 0.0, "rename waited");
}
