//! On the in-order backend a vector store's data waits in the vector
//! store buffer until commit, and a younger scalar load forwards from it.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::config::Console;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, X0};

const SOURCE: u64 = PROGRAM_BASE + 0x400;
const DEST: u64 = SOURCE + 64;
const ELEMENTS: u64 = 8;

/// `vsetivli x0, uimm, e8m1`.
const fn vsetivli_e8(uimm: u32) -> u32 {
    0xC000_0057 | (uimm << 15) | (0b111 << 12)
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: RegIdx) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1.field() << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: RegIdx) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1.field() << 15)
}

/// `vadd.vi vd, vs2, imm`.
const fn vadd_vi(vd: u32, vs2: u32, imm: u32) -> u32 {
    0x0200_0057 | (vd << 7) | (0b011 << 12) | (imm << 15) | (vs2 << 20)
}

/// Loads eight bytes into v1, stores v1 + 1 to `DEST`, reads the stored
/// bytes back with one scalar load, then spins.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        vsetivli_e8(ELEMENTS as u32),
        vle8(1, A0),
        vadd_vi(2, 1, 1),
        vse8(2, A1),
        i().ld(A2, A1, 0).build(),
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn a_scalar_load_forwards_from_an_in_order_vector_store() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::InOrder;
    config.pipeline.width = 4;
    config.cache.l1_d.enabled = true;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
    for byte in 0..ELEMENTS {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, ProbeWidth::B1);
    }

    ctx.run(600);

    assert_eq!(ctx.get_reg(A2), 0x0908_0706_0504_0302, "the stored bytes, forwarded");
    let stored: Vec<u64> = (0..ELEMENTS)
        .map(|b| ctx.sim.probe_mem_load(PhysAddr::new(DEST + b), ProbeWidth::B1))
        .collect();
    assert_eq!(stored, (2..=ELEMENTS + 1).collect::<Vec<_>>(), "the vector store reached memory");
}
