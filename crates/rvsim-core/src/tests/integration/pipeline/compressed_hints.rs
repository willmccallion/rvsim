//! Every compressed HINT runs as a no-op, as a hart that gives it no
//! meaning must: none traps.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const A2: u32 = 12;
const S4: u32 = 20;
const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const MCAUSE: u32 = 0x342;

/// One of each compressed HINT form the C extension lists, in pairs per
/// word.
const HINTS: [u16; 12] = [
    0x0005, // c.nop, nonzero immediate
    0x0081, // c.addi x1, 0
    0x4005, // c.li x0, 1
    0x6005, // c.lui x0, 1
    0x8006, // c.mv x0, x1
    0x9006, // c.add x0, x1
    0x900A, // c.add x0, x2 (c.ntl.p1)
    0x0006, // c.slli x0, 1
    0x0082, // c.slli x1, 0
    0x8001, // c.srli x8, 0
    0x8401, // c.srai x8, 0
    0x0002, // c.slli x0, 0
];

/// The HINTs, a done flag and a spin; a trap goes to a handler that
/// records `mcause` and spins without setting the flag.
fn trap_cause(backend: BackendKind) -> Option<u64> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> =
        HINTS.chunks(2).map(|pair| u32::from(pair[0]) | u32::from(pair[1]) << 16).collect();
    program.extend([i().addi(S4, 0, 1).build(), i().jal(0, 0).build()]);
    let handler = [i().csrrs(A2, MCAUSE, 0).build(), i().jal(0, 0).build()];
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    let finished = ctx.run_until(1_000, |ctx| ctx.get_reg(S4 as usize) == 1);

    finished.is_none().then(|| ctx.get_reg(A2 as usize))
}

#[test]
fn every_compressed_hint_runs_without_trapping() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let trapped = trap_cause(backend);

        assert_eq!(trapped, None, "{backend:?}: trapped with mcause");
    }
}
