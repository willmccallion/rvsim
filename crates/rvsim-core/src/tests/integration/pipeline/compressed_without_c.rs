//! Without C in `misa` every instruction is 32 bits (IALIGN=32): fetch reads
//! a whole word, and one whose low bits are not `11` is illegal rather
//! than a compressed instruction.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const A2: u32 = 12;
const A3: u32 = 13;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const ILLEGAL_INSTRUCTION: u64 = 2;
/// Two `c.nop`s with C, an illegal word without it.
const TWO_C_NOPS: u32 = 0x0001_0001;

/// Records `mcause` and `mtval`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

/// `mcause` and `mtval` after running the two `c.nop`s under `isa`, or
/// zeros when nothing trapped.
fn trap_seen(backend: BackendKind, isa: &str) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.misa_override = Some(isa.parse().expect("a valid ISA string"));
    config.system.console = crate::config::Console::Quiet;
    let program = [TWO_C_NOPS, InstructionBuilder::new().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);

    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize))
}

#[test]
fn a_word_with_low_bits_not_11_is_illegal_without_c() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let seen = trap_seen(backend, "RV64IM");

        assert_eq!(seen, (ILLEGAL_INSTRUCTION, u64::from(TWO_C_NOPS)), "{backend:?}");
    }
}

#[test]
fn the_same_halfwords_run_as_compressed_instructions_with_c() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let seen = trap_seen(backend, "RV64IMC");

        assert_eq!(seen, (0, 0), "{backend:?}");
    }
}
