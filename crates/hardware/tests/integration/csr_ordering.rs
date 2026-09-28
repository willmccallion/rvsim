//! A CSR read must see the CSR write of the instruction before it, on
//! every backend: the kernel's trap entry swaps `tp` with `sscratch` and
//! then reads `sscratch` back, and a stale read leaves `tp` zero.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const MSCRATCH: u32 = 0x340;
const PROGRAM_BASE: u64 = 0x8000_0000;

fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 0x123).build(),
        i().csrrw(0, MSCRATCH, 0).build(),
        i().csrrw(T0, MSCRATCH, T0).build(),
        i().csrrs(T1, MSCRATCH, 0).build(),
        i().csrrw(T2, MSCRATCH, 0).build(),
        i().jal(0, 0).build(),
    ]
}

fn swap_then_read_back(backend: BackendType, width: usize) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    assert_eq!(ctx.get_reg(T0 as usize), 0, "{backend:?} w{width}: the swap returns the old value");
    assert_eq!(
        ctx.get_reg(T1 as usize),
        0x123,
        "{backend:?} w{width}: the read sees the swapped-in value"
    );
    assert_eq!(
        ctx.get_reg(T2 as usize),
        0x123,
        "{backend:?} w{width}: the second swap sees it too"
    );
}

#[test]
fn an_inorder_csr_read_sees_the_preceding_write() {
    swap_then_read_back(BackendType::InOrder, 1);
    swap_then_read_back(BackendType::InOrder, 4);
}

#[test]
fn an_o3_csr_read_sees_the_preceding_write() {
    swap_then_read_back(BackendType::OutOfOrder, 1);
    swap_then_read_back(BackendType::OutOfOrder, 4);
    swap_then_read_back(BackendType::OutOfOrder, 10);
}

/// O3 serializes after a CSR access the way gem5 does: the instruction
/// behind it waits in rename until the ROB drains, instead of being fetched,
/// squashed and fetched again.
#[test]
fn o3_holds_rename_behind_a_csr_access_instead_of_squashing() {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::OutOfOrder;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(400);

    let paths = &ctx.sim.state.cores[0].units.stat_paths.pipeline;
    let stats = &ctx.sim.state.stats;
    assert_eq!(ctx.get_reg(T1 as usize), 0x123, "the read sees the swapped-in value");
    assert_eq!(stats.get(paths.flushes_system), Some(0.0), "no CSR access squashed");
    assert!(stats.get(paths.stalls_serialize).unwrap_or(0.0) > 0.0, "rename waited");
}
