//! A vector store to a device writes each active element as its own
//! access, in element order: a strided store with a zero stride writes the
//! device once per element rather than once for the bytes they share.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, A3, X0};

const SOURCE: u64 = PROGRAM_BASE + 0x400;
const UART_BASE_UPPER: i32 = 0x1_0000;
const MESSAGE: &[u8] = b"rvsim";
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: i32 = 7;

/// `vsetvli x0, rs1, e8m1`.
const fn vsetvli_e8m1(rs1: RegIdx) -> u32 {
    (rs1.field() << 15) | (0b111 << 12) | 0x57
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: RegIdx) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1.field() << 15)
}

/// `vsse8.v vs3, (rs1), rs2`: bytes `rs2` apart.
const fn vsse8(vs3: u32, rs1: RegIdx, rs2: RegIdx) -> u32 {
    (0b10 << 26) | (1 << 25) | (rs2.field() << 20) | (rs1.field() << 15) | (vs3 << 7) | 0x27
}

/// Loads `MESSAGE` into `v3` and stores it to the UART's transmit register
/// with a stride of zero held in a register, which requires every element's
/// access.
fn printing_with_a_zero_stride() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A2, X0, MESSAGE.len() as i32).build(),
        vsetvli_e8m1(A2),
        vle8(3, A0),
        i().lui(A1, UART_BASE_UPPER).build(),
        i().addi(A3, X0, 0).build(),
        vsse8(3, A1, A3),
        i().addi(DONE_REG, X0, DONE).build(),
        JAL_SELF,
    ]
}

fn console_after_printing(backend: BackendKind) -> Vec<u8> {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Captured;
    let mut ctx = TestContext::with_fast_memory(&config)
        .load_program(PROGRAM_BASE, &printing_with_a_zero_stride());
    for (offset, &byte) in (0u64..).zip(MESSAGE) {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + offset), u64::from(byte), ProbeWidth::B1);
    }

    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE as u64).expect("program finished");
    ctx.run(2_000);
    ctx.sim.take_console_output()
}

#[test]
fn a_zero_stride_vector_store_to_a_device_writes_every_element_in_order_o3() {
    assert_eq!(console_after_printing(BackendKind::OutOfOrder), MESSAGE);
}

#[test]
fn a_zero_stride_vector_store_to_a_device_writes_every_element_in_order_inorder() {
    assert_eq!(console_after_printing(BackendKind::InOrder), MESSAGE);
}
