//! Committed scalar and vector stores to the same bytes reach memory in
//! program order, whether the write-combining buffer holds the scalar
//! store or has no room for it yet: a load after both, and memory once
//! they drain, see the younger store.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, A3, A4, A5, X0};

const SOURCE: u64 = PROGRAM_BASE + 0x400;
const DEST: u64 = SOURCE + 0x100;
const OUT: u64 = DEST + 0x100;
const FILLERS: u64 = OUT + 0x100;
const LINE_BYTES: u64 = 64;
const BYTES: u64 = 16;
const ROOMY_WCB_ENTRIES: usize = 16;
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;
const SCALAR_VALUE: i32 = -1;
const SCALAR_BYTES: u64 = u64::MAX;
const VECTOR_BYTES: u64 = 0x0807_0605_0403_0201;

/// `vsetvli x0, rs1, e8m1`.
const fn vsetvli_e8m1(rs1: RegIdx) -> u32 {
    (rs1.field() << 15) | (0b111 << 12) | 0x57
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: RegIdx) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1.field() << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: RegIdx) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1.field() << 15)
}

#[derive(Clone, Copy)]
enum Order {
    ScalarThenVector,
    VectorThenScalar,
}

/// The two stores in `order` and the write-combining buffer they meet.
#[derive(Clone, Copy)]
struct Scenario {
    order: Order,
    wcb_entries: usize,
    /// Stores to other lines just before the two, which fill a small WCB.
    filler_stores: u64,
}

impl Scenario {
    const fn roomy(order: Order) -> Self {
        Self { order, wcb_entries: ROOMY_WCB_ENTRIES, filler_stores: 0 }
    }

    /// The smallest WCB, which the filler stores keep full, so the scalar
    /// store waits in the store buffer for room.
    const fn full_wcb(order: Order) -> Self {
        Self { order, wcb_entries: 2, filler_stores: 6 }
    }
}

/// `count` stores of `A4` to lines of their own past `OUT`.
fn filler_stores(count: u64) -> Vec<u32> {
    let i = InstructionBuilder::new;
    (0..count)
        .flat_map(|n| {
            let offset = (FILLERS + n * LINE_BYTES - SOURCE) as i32;
            [i().addi(A3, A0, offset).build(), i().sd(A3, A4, 0).build()]
        })
        .collect()
}

/// The filler stores, the two stores to `DEST` in `order`, then `ld DEST`
/// with its value stored to `OUT`.
fn program(scenario: Scenario) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let scalar = i().sd(A1, A4, 0).build();
    let vector = vse8(3, A1);
    let stores = match scenario.order {
        Order::ScalarThenVector => [scalar, vector],
        Order::VectorThenScalar => [vector, scalar],
    };
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A2, X0, BYTES as i32).build(),
        vsetvli_e8m1(A2),
        vle8(3, A0),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        i().addi(A4, X0, SCALAR_VALUE).build(),
    ];
    program.extend(filler_stores(scenario.filler_stores));
    program.extend(stores);
    program.extend([
        i().ld(A5, A1, 0).build(),
        i().addi(A3, A0, (OUT - SOURCE) as i32).build(),
        i().sd(A3, A5, 0).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]);
    program
}

/// The value the load read, and `DEST`'s first eight bytes once every
/// store has drained.
fn run(scenario: Scenario, backend: BackendKind) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.cache.wcb_entries = scenario.wcb_entries;
    config.system.console = Console::Quiet;
    let mut ctx =
        TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program(scenario));
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
    let (loaded, memory) = run(Scenario::roomy(Order::ScalarThenVector), BackendKind::OutOfOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_vector_store_after_a_scalar_store_leaves_its_data_inorder() {
    let (loaded, memory) = run(Scenario::roomy(Order::ScalarThenVector), BackendKind::InOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_scalar_store_after_a_vector_store_leaves_its_data_o3() {
    let (loaded, memory) = run(Scenario::roomy(Order::VectorThenScalar), BackendKind::OutOfOrder);

    assert_eq!((loaded, memory), (SCALAR_BYTES, SCALAR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_scalar_store_after_a_vector_store_leaves_its_data_inorder() {
    let (loaded, memory) = run(Scenario::roomy(Order::VectorThenScalar), BackendKind::InOrder);

    assert_eq!((loaded, memory), (SCALAR_BYTES, SCALAR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_vector_store_after_a_scalar_store_waiting_for_the_wcb_leaves_its_data_o3() {
    let (loaded, memory) =
        run(Scenario::full_wcb(Order::ScalarThenVector), BackendKind::OutOfOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}

#[test]
fn a_vector_store_after_a_scalar_store_waiting_for_the_wcb_leaves_its_data_inorder() {
    let (loaded, memory) = run(Scenario::full_wcb(Order::ScalarThenVector), BackendKind::InOrder);

    assert_eq!((loaded, memory), (VECTOR_BYTES, VECTOR_BYTES), "{loaded:#018x} {memory:#018x}");
}
