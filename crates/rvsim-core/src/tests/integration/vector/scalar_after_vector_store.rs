//! A scalar load that follows a scalar store and a younger vector store to
//! the same bytes reads the vector store's data: the youngest older store
//! wins, whichever buffer holds it.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const SOURCE: u64 = PROGRAM_BASE + 0x400;
const DEST: u64 = SOURCE + 0x100;
const OUT: u64 = DEST + 0x100;
const BYTES: u64 = 16;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A4: u32 = 14;
const A5: u32 = 15;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
/// Divides ahead of the stores, keeping them uncommitted in their buffers.
const HEAD_DIVIDES: usize = 8;
/// Multiplies producing the load's address, so it issues after both
/// stores resolve but while the divides still hold them uncommitted.
const ADDRESS_MULTIPLIES: usize = 8;
const SCALAR_VALUE: i32 = -1;

/// `vsetvli x0, rs1, e8m1`.
const fn vsetvli_e8m1(rs1: u32) -> u32 {
    (rs1 << 15) | (0b111 << 12) | 0x57
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: u32) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1 << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: u32) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1 << 15)
}

/// `sd -1, DEST`; `vse8.v` of `SOURCE`'s bytes to `DEST`; `ld DEST`
/// through an address multiplies produce, its value stored to `OUT`.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A2, 0, BYTES as i32).build(),
        vsetvli_e8m1(A2),
        vle8(3, A0),
        i().addi(T0, 0, 1).build(),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        i().addi(A4, 0, SCALAR_VALUE).build(),
        i().addi(T2, 0, 1).build(),
    ];
    program.extend((0..HEAD_DIVIDES).map(|_| i().div(T2, T2, T0).build()));
    program.extend([i().sd(A1, A4, 0).build(), vse8(3, A1), i().addi(T1, A1, 0).build()]);
    program.extend((0..ADDRESS_MULTIPLIES).map(|_| i().mul(T1, T1, T0).build()));
    program.extend([
        i().ld(A5, T1, 0).build(),
        i().addi(A3, A0, (OUT - SOURCE) as i32).build(),
        i().sd(A3, A5, 0).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn loaded(backend: BackendKind) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    for byte in 0..BYTES {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, ProbeWidth::B1);
    }

    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx.run(200);

    ctx.sim.probe_mem_load(PhysAddr::new(OUT), ProbeWidth::B8)
}

const VECTOR_BYTES: u64 = 0x0807_0605_0403_0201;

#[test]
fn load_after_scalar_then_vector_store_forwards_the_vector_data_o3() {
    let value = loaded(BackendKind::OutOfOrder);

    assert_eq!(value, VECTOR_BYTES, "{value:#018x}");
}

#[test]
fn load_after_scalar_then_vector_store_forwards_the_vector_data_inorder() {
    let value = loaded(BackendKind::InOrder);

    assert_eq!(value, VECTOR_BYTES, "{value:#018x}");
}
