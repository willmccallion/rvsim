//! An LR/SC spinlock guards a plain read-modify-write counter; mutual
//! exclusion must hold, so the counter equals the total iteration count.

use crate::common::builder::instruction::{ECALL, FENCE_IORW, InstructionBuilder};
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
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A7: u32 = 17;
const MHARTID: u32 = 0xF14;
const SYS_EXIT: i32 = 93;

/// Lock at `data[0]`, counter at `data[1]`, arrival count at `data[2]`.
fn program(harts: i32, iterations: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().addi(T1, 0, iterations).build(),
        i().addi(T3, 0, 1).build(),
        i().addi(T4, T2, 16).build(),
        i().lr_d(T5, T2).build(),
        i().bne(T5, 0, -4).build(),
        i().sc_d(T6, T2, T3).build(),
        i().bne(T6, 0, -12).build(),
        i().ld(T6, T2, 8).build(),
        i().addi(T6, T6, 1).build(),
        i().sd(T2, T6, 8).build(),
        FENCE_IORW,
        i().sd(T2, 0, 0).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -40).build(),
        i().amoadd_d(0, T4, T3).build(),
        i().csrrs(A1, MHARTID, 0).build(),
        i().bne(A1, 0, 28).build(),
        i().ld(A2, T2, 16).build(),
        i().addi(A3, 0, harts).build(),
        i().bne(A2, A3, -8).build(),
        i().ld(A0, T2, 8).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().jal(0, 0).build(),
    ]
}

fn critical_section_is_exclusive(harts: usize, backend: BackendType) {
    let iterations = 100;
    let mut system = MultiHart::new(harts, backend, &program(harts as i32, iterations));

    let exit = system.run_until_exit(8_000_000);

    let expected = (harts as u64) * (iterations as u64);
    assert_eq!(exit, Some(expected), "{harts} harts on {backend:?}");
    assert_eq!(system.read_u64(DATA_BASE), 0, "lock released at the end");
}

#[test]
fn two_inorder_harts_never_share_the_critical_section() {
    critical_section_is_exclusive(2, BackendType::InOrder);
}

#[test]
fn four_inorder_harts_never_share_the_critical_section() {
    critical_section_is_exclusive(4, BackendType::InOrder);
}

#[test]
fn two_o3_harts_never_share_the_critical_section() {
    critical_section_is_exclusive(2, BackendType::OutOfOrder);
}

#[test]
fn four_o3_harts_never_share_the_critical_section() {
    critical_section_is_exclusive(4, BackendType::OutOfOrder);
}
