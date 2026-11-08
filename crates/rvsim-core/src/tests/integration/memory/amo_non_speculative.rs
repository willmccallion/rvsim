//! An AMO takes effect in the cache, so it executes only as the oldest.
//!
//! One on a mispredicted path never reaches memory.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T3, T4, X0};

const DATA: u64 = PROGRAM_BASE + 0x400;
const BASE: RegIdx = RegIdx::new(5);
const ONE: RegIdx = RegIdx::new(6);
const SLOW: RegIdx = RegIdx::new(7);
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;

/// A branch that is taken once a divide finishes, skipping an `amoadd`
/// the front end fetches down the fall-through path meanwhile.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(BASE, 0).build(),
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(ONE, X0, 1).build(),
        i().div(SLOW, ONE, ONE).build(),
        i().bne(SLOW, X0, 8).build(),
        i().amoadd_d(X0, BASE, ONE).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn an_amo_on_a_mispredicted_path_never_reaches_memory() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut config = Config::minimal();
        config.pipeline.backend = backend;
        config.pipeline.width = 4;
        config.cache.l1_d.enabled = true;
        let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());

        ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");

        let paths = ctx.sim.state.cores[0].units.stat_paths.bp;
        let mispredicts =
            ctx.sim.state.uncore.kernel.stats.get(paths.committed_mispredicts).unwrap_or(0.0);
        assert!(mispredicts >= 1.0, "{backend:?}: the branch was predicted not taken");
        assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(DATA), ProbeWidth::B8), 0, "{backend:?}");
    }
}

const MTVEC: u32 = 0x305;
const MEPC: u32 = 0x341;
const MRET: u32 = 0x3020_0073;
const ILLEGAL: u32 = 0;

/// An illegal instruction with an `amoadd` behind it, and a handler that
/// steps over the illegal instruction: the `amoadd` runs once, after it.
fn amo_behind_a_trap() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(BASE, 0).build(),                                 // 0x00
        i().addi(BASE, BASE, (DATA - PROGRAM_BASE) as i32).build(), // 0x04
        i().addi(ONE, X0, 1).build(),                               // 0x08
        i().auipc(T3, 0).build(),                                   // 0x0c
        i().addi(T3, T3, 0x24).build(),                             // 0x10: the handler
        i().csrrw(X0, MTVEC, T3).build(),                           // 0x14
        ILLEGAL,                                                    // 0x18
        i().amoadd_d(X0, BASE, ONE).build(),                        // 0x1c
        i().addi(DONE_REG, X0, DONE as i32).build(),                // 0x20
        i().jal(X0, 0).build(),                                     // 0x24
        i().nop().build(),                                          // 0x28
        i().nop().build(),                                          // 0x2c
        i().csrrs(T4, MEPC, X0).build(),                            // 0x30: handler
        i().addi(T4, T4, 4).build(),                                // 0x34
        i().csrrw(X0, MEPC, T4).build(),                            // 0x38
        MRET,                                                       // 0x3c
    ]
}

#[test]
fn an_amo_behind_a_trap_takes_effect_once() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut config = Config::minimal();
        config.pipeline.backend = backend;
        config.pipeline.width = 4;
        config.pipeline.trap_latency = 13;
        config.cache.l1_d.enabled = true;
        let mut ctx =
            TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &amo_behind_a_trap());

        ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");

        assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(DATA), ProbeWidth::B8), 1, "{backend:?}");
    }
}
