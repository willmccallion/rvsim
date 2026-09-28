//! A committed vector store reaches the L1D one line at a time: the vector
//! store buffer writes each line's bytes as a single masked line write.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const SOURCE: u64 = PROGRAM_BASE + 0x400;
const DEST: u64 = SOURCE + 0x100;
const BYTES: u64 = 64;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const DONE_REG: usize = 31;
const DONE: u64 = 7;

/// `vsetvli x0, rs1, e8m4`.
const fn vsetvli_e8m4(rs1: u32) -> u32 {
    (0b010 << 20) | (rs1 << 15) | (0b111 << 12) | 0x57
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: u32) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1 << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: u32) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1 << 15)
}

/// Copies `BYTES` bytes from `SOURCE` to `DEST` with one vector load and,
/// when `with_store`, one vector store (else a `nop` in its place).
fn copy(with_store: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let store = if with_store { vse8(4, A1) } else { i().nop().build() };
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        i().addi(A2, 0, BYTES as i32).build(),
        vsetvli_e8m4(A2),
        vle8(4, A0),
        store,
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
    ];
    program.push(i().jal(0, 0).build());
    program
}

/// L1D accesses of `copy(with_store)`, and the context after it finished.
fn l1d_accesses(backend: BackendType, with_store: bool) -> (u64, TestContext) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.cache.l1_d.enabled = true;
    config.system.uart_quiet = true;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &copy(with_store));
    for byte in 0..BYTES {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, 1);
    }
    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    // The committed store drains after the marker retires.
    ctx.run(200);
    let stats = &ctx.sim.state.stats;
    let accesses = ["core0.cache.l1d.hits", "core0.cache.l1d.misses"]
        .iter()
        .map(|path| stats.get(path).unwrap_or(0.0))
        .sum::<f64>();
    (accesses as u64, ctx)
}

#[test]
fn a_vector_store_writes_its_line_to_the_l1d_in_one_access() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let (with_store, mut ctx) = l1d_accesses(backend, true);
        let (without_store, _) = l1d_accesses(backend, false);

        assert_eq!(with_store - without_store, 1, "{backend:?}: one line, one write");
        let stored: Vec<u64> =
            (0..BYTES).map(|b| ctx.sim.probe_mem_load(PhysAddr::new(DEST + b), 1)).collect();
        assert_eq!(
            stored,
            (1..=BYTES).collect::<Vec<_>>(),
            "{backend:?}: the bytes reached memory"
        );
    }
}
