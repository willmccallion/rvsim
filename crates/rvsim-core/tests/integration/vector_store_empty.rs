//! A vector store with no active elements (`vl` = 0) still frees its
//! vector store buffer entry when it retires, so any number of them leave
//! room for the stores that follow.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::{Config, Console};

const PROGRAM_BASE: u64 = 0x8000_0000;
const SOURCE: u64 = PROGRAM_BASE + 0x400;
const DEST: u64 = SOURCE + 64;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const ELEMENTS: u64 = 8;
const VSB_ENTRIES: usize = 4;

/// `vsetivli x0, uimm, e8m1`.
const fn vsetivli_e8(uimm: u32) -> u32 {
    0xC000_0057 | (uimm << 15) | (0b111 << 12)
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: u32) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1 << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: u32) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1 << 15)
}

/// More `vl` = 0 stores than the buffer has entries, then a real store of
/// eight bytes, read back with a scalar load.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut words = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        vsetivli_e8(ELEMENTS as u32),
        vle8(1, A0),
        vsetivli_e8(0),
    ];
    words.extend(std::iter::repeat_n(vse8(1, A1), 2 * VSB_ENTRIES));
    words.extend([vsetivli_e8(ELEMENTS as u32), vse8(1, A1), i().ld(A2, A1, 0).build()]);
    words.push(i().jal(0, 0).build());
    words
}

fn run(backend: BackendKind) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.pipeline.vec_store_buffer_size = VSB_ENTRIES;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    for byte in 0..ELEMENTS {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, 1);
    }

    ctx.run(5_000);

    ctx.get_reg(A2 as usize)
}

#[test]
fn empty_vector_stores_free_their_buffer_entries_on_the_in_order_core() {
    assert_eq!(run(BackendKind::InOrder), 0x0807_0605_0403_0201);
}

#[test]
fn empty_vector_stores_free_their_buffer_entries_on_the_o3_core() {
    assert_eq!(run(BackendKind::OutOfOrder), 0x0807_0605_0403_0201);
}
