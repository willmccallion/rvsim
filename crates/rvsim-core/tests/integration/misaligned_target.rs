//! Taken branches and jumps to targets IALIGN does not allow.
//!
//! Without the C extension (IALIGN=32) a taken branch or jump to an address
//! that is not four-byte aligned raises an instruction-address-misaligned
//! exception on the branch or jump itself; a branch not taken does not.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x10_000;
const TRAP_PARK: u64 = RAM_BASE + 0x800;
const JAL_SELF: u32 = 0x0000_006F;
const LINK: usize = 1;
const LINK_BEFORE: u64 = 0x77;
const MISALIGNED: u64 = 0;
/// Offset of the control-transfer instruction in every program below.
const CTI_OFFSET: u64 = 8;

struct Outcome {
    mcause: u64,
    mepc: u64,
    mtval: u64,
    link: u64,
}

/// Runs `program` (then `j .`) in machine mode on a hart without C.
fn run(backend: BackendKind, program: &[u32]) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.misa_override = Some("RV64IMAFD".parse().expect("valid ISA string"));
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().chain(&[JAL_SELF]).enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + 4 * n as u64), u64::from(*inst), 4);
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), 4);
    ctx.sim.state.harts[0].csrs.mtvec = TRAP_PARK;
    ctx.sim.state.harts[0].pc = RAM_BASE;
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    let csrs = &ctx.sim.state.harts[0].csrs;
    Outcome { mcause: csrs.mcause, mepc: csrs.mepc, mtval: csrs.mtval, link: ctx.get_reg(LINK) }
}

fn prologue() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().addi(LINK as u32, 0, LINK_BEFORE as i32).build(), i().addi(5, 0, 1).build()]
}

fn with_cti(cti: u32) -> Vec<u32> {
    let mut program = prologue();
    program.push(cti);
    program
}

fn assert_traps_on_the_cti(backend: BackendKind, cti: u32, target_offset: u64) {
    let outcome = run(backend, &with_cti(cti));

    assert_eq!(outcome.mcause, MISALIGNED, "{backend:?}: instruction-address-misaligned");
    assert_eq!(outcome.mepc, RAM_BASE + CTI_OFFSET, "{backend:?}: reported on the jump");
    assert_eq!(outcome.mtval, RAM_BASE + target_offset, "{backend:?}: tval is the target");
    assert_eq!(outcome.link, LINK_BEFORE, "{backend:?}: the jump wrote no link");
}

#[test]
fn a_jal_to_a_halfword_aligned_target_traps_on_the_jal() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let jal = InstructionBuilder::new().jal(LINK as u32, 6).build();
        assert_traps_on_the_cti(backend, jal, CTI_OFFSET + 6);
    }
}

#[test]
fn a_jalr_to_a_halfword_aligned_target_traps_on_the_jalr() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let i = InstructionBuilder::new;
        let program = [
            i().addi(LINK as u32, 0, LINK_BEFORE as i32).build(),
            i().auipc(6, 0).build(),
            i().jalr(LINK as u32, 6, 0x12).build(),
        ];
        let outcome = run(backend, &program);

        assert_eq!(outcome.mcause, MISALIGNED, "{backend:?}");
        assert_eq!(outcome.mepc, RAM_BASE + CTI_OFFSET, "{backend:?}");
        assert_eq!(outcome.mtval, RAM_BASE + 4 + 0x12, "{backend:?}");
        assert_eq!(outcome.link, LINK_BEFORE, "{backend:?}");
    }
}

#[test]
fn a_taken_branch_to_a_halfword_aligned_target_traps_on_the_branch() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let beq = InstructionBuilder::new().beq(0, 0, 6).build();
        assert_traps_on_the_cti(backend, beq, CTI_OFFSET + 6);
    }
}

#[test]
fn a_branch_not_taken_to_a_misaligned_target_does_not_trap() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let bne = InstructionBuilder::new().bne(0, 0, 6).build();
        let outcome = run(backend, &with_cti(bne));

        assert_eq!((outcome.mcause, outcome.mepc), (0, 0), "{backend:?}: no trap");
    }
}
