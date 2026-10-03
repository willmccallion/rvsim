//! A load forwarded at zero latency carries the store's value, extended
//! as the load's width asks, on both backends.

use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;
const DATA_OFFSET: i32 = 0x400;
const DONE_REG: usize = 2;
const DONE_VALUE: u64 = 7;

fn config(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.store_forward_latency = Some(0);
    config.cache.l1_d.enabled = true;
    config
}

/// Stores `-1` as a doubleword and loads it back as a word, then stores
/// `5` and loads it back as a doubleword.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, DATA_OFFSET).build(),
        i().addi(5, 0, -1).build(),
        i().sd(10, 5, 0).build(),
        i().lw(6, 10, 0).build(),
        i().addi(7, 0, 5).build(),
        i().sd(10, 7, 8).build(),
        i().ld(8, 10, 8).build(),
        i().addi(DONE_REG as u32, 0, DONE_VALUE as i32).build(),
    ];
    program.extend(std::iter::repeat_n(i().nop().build(), 4));
    program
}

fn assert_forwarded_values_are_the_stored_ones(backend: BackendKind) {
    let mut tc = TestContext::new_with_config(&config(backend))
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, &program());

    tc.run_until(10_000, |tc| tc.get_reg(DONE_REG) == DONE_VALUE).expect("program did not finish");

    assert_eq!(tc.get_reg(6), u64::MAX, "{backend:?}: lw sign-extends the forwarded word");
    assert_eq!(tc.get_reg(8), 5, "{backend:?}: ld reads the forwarded doubleword");
}

#[test]
fn inorder_zero_latency_forward_delivers_the_stored_value() {
    assert_forwarded_values_are_the_stored_ones(BackendKind::InOrder);
}

#[test]
fn o3_zero_latency_forward_delivers_the_stored_value() {
    assert_forwarded_values_are_the_stored_ones(BackendKind::OutOfOrder);
}
