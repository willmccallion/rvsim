//! The out-of-order backend's integer and FP register files: rename gives
//! a destination a register of its own file, and stalls when that file has
//! none free, whatever the other file has.

use crate::config::{BackendKind, Config};
use crate::isa::reg::{ArchReg, RegIdx};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{X0, X1, X3, X4, X30, after};
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rename::scalar_free::ScalarFreeLists;

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;
const DONE_REG: RegIdx = RegIdx::new(2);
const DONE_VALUE: u64 = 7;
/// Cycles the multiply at the head of the window holds it.
const MUL_LATENCY: u64 = 40;
/// Independent integer writes behind the multiply.
const ADDS: usize = 12;
const ROOMY: usize = 32 + 64;
const SCARCE: usize = 32 + 4;

/// An out-of-order core with `gpr` integer and `fpr` FP registers.
fn config(gpr: usize, fpr: usize) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.fu_config.int_mul_latency = MUL_LATENCY;
    config.pipeline.prf_gpr_size = gpr;
    config.pipeline.prf_fpr_size = fpr;
    config
}

/// A slow multiply, `ADDS` integer writes that do not wait for it, and a
/// second slow multiply that overlaps the first only if it renames before
/// the first retires.
fn adds_between_multiplies() -> Vec<u32> {
    let mut program = vec![
        InstructionBuilder::new().addi(X1, X0, 3).build(),
        InstructionBuilder::new().mul(X3, X1, X1).build(),
    ];
    program.extend((0..ADDS).map(|i| InstructionBuilder::new().addi(after(X4, i), X0, 1).build()));
    program.push(InstructionBuilder::new().mul(X30, X1, X1).build());
    program.push(InstructionBuilder::new().addi(DONE_REG, X0, DONE_VALUE as i32).build());
    program.extend(std::iter::repeat_n(InstructionBuilder::new().nop().build(), 4));
    program
}

fn cycles_to_finish(config: &Config) -> u64 {
    let mut tc = TestContext::with_fast_memory(config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, &adds_between_multiplies());
    tc.run_until(5_000, |tc| tc.get_reg(DONE_REG) == DONE_VALUE).expect("program did not finish")
}

#[test]
fn integer_writes_stall_when_the_integer_file_runs_out() {
    let roomy = cycles_to_finish(&config(ROOMY, ROOMY));
    let scarce_int = cycles_to_finish(&config(SCARCE, ROOMY));

    assert!(
        scarce_int >= roomy + MUL_LATENCY / 2,
        "{scarce_int} cycles with 4 integer renames against {roomy} with 64: \
         the second multiply should wait for the first to retire"
    );
}

#[test]
fn integer_writes_do_not_use_the_fp_file() {
    let roomy = cycles_to_finish(&config(ROOMY, ROOMY));
    let scarce_fp = cycles_to_finish(&config(ROOMY, SCARCE));

    assert_eq!(scarce_fp, roomy, "a small FP file should not slow integer code");
}

#[test]
fn each_file_hands_out_only_its_own_registers() {
    let mut lists = ScalarFreeLists::new(32 + 2, 32 + 1);
    let int = ArchReg::Int(RegIdx::new(5));
    let fp = ArchReg::Fp(RegIdx::new(5));

    let ints = [lists.allocate(int), lists.allocate(int)];
    let fps = [lists.allocate(fp)];

    assert_eq!(ints, [Some(PhysReg::from_index(64)), Some(PhysReg::from_index(65))]);
    assert_eq!(fps, [Some(PhysReg::from_index(66))]);
    assert_eq!(lists.allocate(int), None);
    assert_eq!(lists.allocate(fp), None);
}

#[test]
fn a_freed_register_returns_to_its_own_file() {
    let mut lists = ScalarFreeLists::new(32 + 1, 32 + 1);
    let int = ArchReg::Int(RegIdx::new(5));
    let fp = ArchReg::Fp(RegIdx::new(5));
    let taken = (lists.allocate(int), lists.allocate(fp));

    // f8's architectural register, freed when a rename of f8 retires.
    lists.reclaim(PhysReg::from_index(32 + 8));

    assert!(taken.0.is_some() && taken.1.is_some());
    assert!(lists.has_free(fp));
    assert!(!lists.has_free(int));
}

#[test]
fn the_zero_register_is_never_freed() {
    let mut lists = ScalarFreeLists::new(32 + 1, 32 + 1);

    lists.reclaim(PhysReg::ZERO);

    assert_eq!(lists.available(), 2);
}

#[test]
fn an_empty_fp_file_has_none_free_while_the_integer_file_does() {
    let mut lists = ScalarFreeLists::new(32 + 1, 32 + 1);
    let taken = lists.allocate(ArchReg::Fp(RegIdx::new(5)));

    assert!(taken.is_some());
    assert!(!lists.has_free(ArchReg::Fp(RegIdx::new(5))));
    assert!(lists.has_free(ArchReg::Int(RegIdx::new(5))));
}

#[test]
fn the_first_integer_rename_register_returns_to_the_integer_file() {
    let mut lists = ScalarFreeLists::new(32 + 1, 32 + 1);
    let int = ArchReg::Int(RegIdx::new(5));
    let first = lists.allocate(int).expect("one integer register is free");

    lists.reclaim(first);

    assert_eq!(first, PhysReg::from_index(64));
    assert!(lists.has_free(int));
}
