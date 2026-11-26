//! Committed stores drain whatever commit is doing: a hart waiting in WFI,
//! or waiting out the latency of a trap, still writes the stores it
//! committed before, so other harts and devices see them.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T0, X0};

const WFI: u32 = 0x1050_0073;
const ECALL: u32 = 0x0000_0073;
const HANDLER_OFFSET: u64 = 0x200;
/// Cycles from commit finding the `ecall` to taking it: far more than the
/// stores need.
const TRAP_LATENCY: u64 = 100;
const DATA_OFFSET: i32 = 0x400;
const LINE_BYTES: i32 = 64;
/// More stores than the store buffer sends before the WFI retires: four
/// commit a cycle, and it sends one a cycle.
const STORES: i32 = 8;
const WIDTH: usize = 4;
const CYCLE_LIMIT: u64 = 2_000;
/// `mie.MEIE`: an interrupt is enabled, so the WFI waits, but nothing
/// here raises one, so the wait never ends.
const MIE_MEIE: u64 = 1 << 11;

/// Stores its own address to each of `STORES` lines, then runs `then`.
fn store_then(then: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().auipc(T0, 0).build(), i().addi(T0, T0, DATA_OFFSET).build()];
    program.extend((0..STORES).map(|line| i().sd(T0, T0, line * LINE_BYTES).build()));
    program.extend([then, i().jal(X0, 0).build()]);
    program
}

fn context(backend: BackendKind, program: &[u32]) -> TestContext {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = WIDTH;
    config.pipeline.trap_latency = TRAP_LATENCY;
    config.system.console = Console::Quiet;
    TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program)
}

fn stores_written(ctx: &mut TestContext) -> bool {
    let data = PROGRAM_BASE + DATA_OFFSET as u64;
    (0..STORES as u64).all(|line| {
        let address = PhysAddr::new(data + line * LINE_BYTES as u64);
        ctx.sim.probe_mem_load(address, ProbeWidth::B8) == data
    })
}

#[test]
fn stores_committed_before_a_wfi_reach_memory_while_the_hart_waits() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = context(backend, &store_then(WFI));
        ctx.sim.state.harts[0].csrs.mie = MIE_MEIE;
        ctx.run_until(CYCLE_LIMIT, |ctx| ctx.state().harts[0].wfi_waiting)
            .expect("the hart reaches its WFI");
        let pending_at_wait = !stores_written(&mut ctx);

        let written = (0..CYCLE_LIMIT).any(|_| {
            ctx.run(1);
            stores_written(&mut ctx)
        });

        assert!(pending_at_wait, "{backend:?}: some stores were still buffered at the WFI");
        assert!(written, "{backend:?}: the buffered stores were never written");
        assert!(ctx.sim.state.harts[0].wfi_waiting, "{backend:?}: the hart still waits");
    }
}

#[test]
fn stores_committed_before_a_trap_reach_memory_before_it_is_taken() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = context(backend, &store_then(ECALL));
        let handler = PROGRAM_BASE + HANDLER_OFFSET;
        ctx.sim.probe_mem_store(
            PhysAddr::new(handler),
            u64::from(InstructionBuilder::new().jal(X0, 0).build()),
            ProbeWidth::B4,
        );
        ctx.sim.state.harts[0].csrs.mtvec = handler;

        ctx.run_until(CYCLE_LIMIT, |ctx| ctx.state().harts[0].csrs.mcause != 0)
            .expect("the ecall traps");

        assert!(stores_written(&mut ctx), "{backend:?}: the stores were written before the trap");
    }
}
