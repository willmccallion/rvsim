//! An instruction that faults at execute takes that fault and nothing else.
//!
//! A floating-point store with `mstatus.FS` off is illegal; it must not go
//! on to compute an address, touch memory, or report a later stage's fault.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const DATA: u64 = PROGRAM_BASE + 0x200;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const T0: u32 = 5;
const MSTATUS: u32 = 0x300;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const ILLEGAL_INSTRUCTION: u64 = 2;
/// `fsw ft0, 0(a1)`.
const FSW_FT0_A1: u32 = 0x0005_A027;

/// Clears `mstatus.FS`, then stores `ft0` through `a1` and spins.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(T0, 0x6).build(),
        i().csrrc(0, MSTATUS, T0).build(),
        i().auipc(A1, 0).build(),
        i().addi(A1, A1, (DATA - PROGRAM_BASE - 8) as i32).build(),
        i().addi(A3, 0, 7).build(),
        FSW_FT0_A1,
        i().jal(0, 0).build(),
    ]
}

/// Records `mcause` and `mtval`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

fn store_program(ctx: &mut TestContext, base: u64, program: &[u32]) {
    for (i, word) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(base + (i as u64) * 4), u64::from(*word), 4);
    }
}

fn run(backend: BackendType) -> (u64, u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    store_program(&mut ctx, HANDLER, &handler());
    ctx.sim.probe_mem_store(PhysAddr::new(DATA), 0x1234_5678, 4);
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    let memory = ctx.sim.probe_mem_load(PhysAddr::new(DATA), 4);
    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize), memory)
}

fn check(backend: BackendType) {
    let (mcause, mtval, memory) = run(backend);

    assert_eq!(mcause, ILLEGAL_INSTRUCTION, "{backend:?}: the FS=0 fault is the one taken");
    assert_eq!(mtval, FSW_FT0_A1 as u64, "{backend:?}: tval holds the illegal instruction");
    assert_eq!(memory, 0x1234_5678, "{backend:?}: the store never reached memory");
}

#[test]
fn an_fp_store_with_fs_off_takes_the_illegal_instruction_fault_inorder() {
    check(BackendType::InOrder);
}

#[test]
fn an_fp_store_with_fs_off_takes_the_illegal_instruction_fault_o3() {
    check(BackendType::OutOfOrder);
}
