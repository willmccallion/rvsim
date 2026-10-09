//! On the out-of-order backend a vector load that reads memory before an
//! older vector store has resolved its address still returns the store's
//! data: the store finds the load's micro-ops in the load queue and
//! squashes it. A load with more micro-ops than load-queue slots waits
//! until no older memory access is in flight.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
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
const T0: u32 = 5;
const T1: u32 = 6;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const DIVIDES: usize = 8;

/// `vsetvli x0, rs1, e8m1`.
const fn vsetvli_e8m1(rs1: u32) -> u32 {
    (rs1 << 15) | (0b111 << 12) | 0x57
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: u32) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1 << 15)
}

/// `vluxei8.v vd, (rs1), vs2`.
const fn vluxei8(vd: u32, rs1: u32, vs2: u32) -> u32 {
    (0b01 << 26) | (1 << 25) | (vs2 << 20) | (rs1 << 15) | (vd << 7) | 0x07
}

/// `vid.v vd`.
const fn vid(vd: u32) -> u32 {
    (0b01_0100 << 26) | (1 << 25) | (0b1_0001 << 15) | (0b010 << 12) | (vd << 7) | 0x57
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: u32) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1 << 15)
}

/// Stores `SOURCE`'s bytes to `DEST` through an address a chain of divides
/// produces, then loads `DEST` with `load` (unit-stride, or indexed by
/// element number), which needs no divide, and copies what it read to `OUT`.
fn store_then_load(load: Load) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A2, 0, BYTES as i32).build(),
        vsetvli_e8m1(A2),
        vle8(3, A0),
        vid(2),
        i().addi(T0, 0, 1).build(),
        i().addi(T1, A0, (DEST - SOURCE) as i32).build(),
    ];
    program.extend((0..DIVIDES).map(|_| i().div(T1, T1, T0).build()));
    program.extend([
        vse8(3, T1),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        match load {
            Load::UnitStride => vle8(4, A1),
            Load::Indexed => vluxei8(4, A1, 2),
        },
        i().addi(A3, A0, (OUT - SOURCE) as i32).build(),
        vse8(4, A3),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

#[derive(Clone, Copy)]
enum Load {
    UnitStride,
    Indexed,
}

/// What `store_then_load(load)` copied to `OUT` on the out-of-order
/// backend with `load_queue_size` load-queue slots.
fn copied(load: Load, load_queue_size: usize) -> Vec<u64> {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.load_queue_size = load_queue_size;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &store_then_load(load));
    for byte in 0..BYTES {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, 1);
    }

    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx.run(200);

    (0..BYTES).map(|b| ctx.sim.probe_mem_load(PhysAddr::new(OUT + b), 1)).collect()
}

#[test]
fn a_vector_load_reads_an_older_vector_store_whose_address_came_late() {
    let copied = copied(Load::UnitStride, 32);

    assert_eq!(copied, (1..=BYTES).collect::<Vec<_>>());
}

#[test]
fn a_load_with_more_elements_than_load_queue_slots_reads_the_older_store() {
    let copied = copied(Load::Indexed, 2);

    assert_eq!(copied, (1..=BYTES).collect::<Vec<_>>());
}
