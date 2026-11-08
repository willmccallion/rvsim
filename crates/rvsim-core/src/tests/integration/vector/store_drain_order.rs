//! Committed scalar and vector stores to the same bytes reach memory in
//! program order when the write-combining buffer holds the scalar ones:
//! a load after both, and memory once they drain, see the younger store.

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
const WCB_ENTRIES: usize = 16;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A4: u32 = 14;
const A5: u32 = 15;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const SCALAR_VALUE: i32 = -1;
const SCALAR_BYTES: u64 = u64::MAX;
const VECTOR_BYTES: u64 = 0x0807_0605_0403_0201;

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

#[derive(Clone, Copy)]
enum Order {
    ScalarThenVector,
    VectorThenScalar,
}

/// The two stores to `DEST` in `order`, then `ld DEST` with its value
/// stored to `OUT`.
fn program(order: Order) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let scalar = i().sd(A1, A4, 0).build();
    let vector = vse8(3, A1);
    let stores = match order {
        Order::ScalarThenVector => [scalar, vector],
        Order::VectorThenScalar => [vector, scalar],
    };
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A2, 0, BYTES as i32).build(),
        vsetvli_e8m1(A2),
        vle8(3, A0),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        i().addi(A4, 0, SCALAR_VALUE).build(),
    ];
    program.extend(stores);
    program.extend([
        i().ld(A5, A1, 0).build(),
        i().addi(A3, A0, (OUT - SOURCE) as i32).build(),
        i().sd(A3, A5, 0).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

/// The value the load read, and `DEST`'s first eight bytes once every
/// store has drained.
fn run(order: Order, backend: BackendKind) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.cache.wcb_entries = WCB_ENTRIES;
    config.system.console = Console::Quiet;
    let mut ctx =
        TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program(order));
    for byte in 0..BYTES {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, ProbeWidth::B1);
    }

    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx.run(2_000);

    (
        ctx.sim.probe_mem_load(PhysAddr::new(OUT), ProbeWidth::B8),
        ctx.sim.probe_mem_load(PhysAddr::new(DEST), ProbeWidth::B8),
    )
}

#[test]
fn a_vector_store_after_a_scalar_store_leaves_its_data_o3() {
    let (loaded, memory) = run(Order::ScalarThenVector, BackendKind::OutOfOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_vector_store_after_a_scalar_store_leaves_its_data_inorder() {
    let (loaded, memory) = run(Order::ScalarThenVector, BackendKind::InOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_scalar_store_after_a_vector_store_leaves_its_data_o3() {
    let (loaded, memory) = run(Order::VectorThenScalar, BackendKind::OutOfOrder);

    assert_eq!((loaded, memory), (SCALAR_BYTES, SCALAR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_scalar_store_after_a_vector_store_leaves_its_data_inorder() {
    let (loaded, memory) = run(Order::VectorThenScalar, BackendKind::InOrder);

    assert_eq!((loaded, memory), (SCALAR_BYTES, SCALAR_BYTES), "{loaded:#018x} {memory:#018x}");
}
