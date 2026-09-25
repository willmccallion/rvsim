//! Every hart adds to one shared word with `amoadd.d`; the total must be
//! exact however the harts interleave.

use crate::common::builder::instruction::{ECALL, InstructionBuilder};
use crate::common::multihart::{DATA_BASE, MultiHart};
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;
const T5: u32 = 30;
const T6: u32 = 31;
const A0: u32 = 10;
const A2: u32 = 12;
const A7: u32 = 17;
const MHARTID: u32 = 0xF14;
const SYS_EXIT: i32 = 93;

/// `data[0] += 1` `iterations` times on every hart, then `data[1] += 1`;
/// hart 0 waits for `data[1] == harts` and exits with `data[0]`.
fn program(harts: i32, iterations: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().addi(T1, 0, iterations).build(),
        i().addi(T3, 0, 1).build(),
        i().addi(T4, T2, 8).build(),
        i().amoadd_d(0, T2, T3).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -8).build(),
        i().amoadd_d(0, T4, T3).build(),
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, 28).build(),
        i().ld(T6, T2, 8).build(),
        i().addi(A2, 0, harts).build(),
        i().bne(T6, A2, -8).build(),
        i().ld(A0, T2, 0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().jal(0, 0).build(),
    ]
}

fn total_is_exact(harts: usize, backend: BackendType) {
    let iterations = 200;
    let mut system = MultiHart::new(harts, backend, &program(harts as i32, iterations));

    let exit = system.run_until_exit(4_000_000);

    let expected = (harts as u64) * (iterations as u64);
    assert_eq!(exit, Some(expected), "{harts} harts on {backend:?}");
    assert_eq!(system.read_u64(DATA_BASE), expected);
    assert_eq!(system.read_u64(DATA_BASE + 8), harts as u64);
}

#[test]
fn two_inorder_harts_count_exactly() {
    total_is_exact(2, BackendType::InOrder);
}

#[test]
fn four_inorder_harts_count_exactly() {
    total_is_exact(4, BackendType::InOrder);
}

#[test]
fn two_o3_harts_count_exactly() {
    total_is_exact(2, BackendType::OutOfOrder);
}

#[test]
fn four_o3_harts_count_exactly() {
    total_is_exact(4, BackendType::OutOfOrder);
}

#[test]
fn a_single_hart_has_no_write_log() {
    let mut system = MultiHart::new(1, BackendType::OutOfOrder, &program(1, 50));
    assert!(system.sim.state.write_log.is_none());
    assert_eq!(system.run_until_exit(1_000_000), Some(50));
}
