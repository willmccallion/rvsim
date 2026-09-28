//! A unit-stride vector access moves its elements a datapath-width window
//! at a time: one L1D access per window, taken apart into elements only
//! where one of them faults.

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
const T0: u32 = 5;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const MTVEC: u32 = 0x305;
const LOAD_ACCESS_FAULT: u64 = 5;
const BACKENDS: [BackendType; 2] = [BackendType::InOrder, BackendType::OutOfOrder];

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

/// Copies `BYTES` bytes from `SOURCE` to `DEST` through v4..v7; without
/// `with_load` a `nop` stands in for the load.
fn copy(with_load: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let load = if with_load { vle8(4, A0) } else { i().nop().build() };
    vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (SOURCE - PROGRAM_BASE) as i32).build(),
        i().addi(A1, A0, (DEST - SOURCE) as i32).build(),
        i().addi(A2, 0, BYTES as i32).build(),
        vsetvli_e8m4(A2),
        load,
        vse8(4, A1),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

fn config(backend: BackendType, vector_mem_width: Option<usize>) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.vector_mem_width = vector_mem_width;
    config.cache.l1_d.enabled = true;
    config.system.console = rvsim_core::config::Console::Quiet;
    config
}

/// L1D accesses of `copy(with_load)`, and the context after it finished.
fn l1d_accesses(config: &Config, with_load: bool) -> (u64, TestContext) {
    let mut ctx = TestContext::new_with_config(config).load_program(PROGRAM_BASE, &copy(with_load));
    for byte in 0..BYTES {
        ctx.sim.probe_mem_store(PhysAddr::new(SOURCE + byte), byte + 1, 1);
    }
    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    ctx.run(200);
    let stats = &ctx.sim.state.stats;
    let accesses = ["core0.cache.l1d.hits", "core0.cache.l1d.misses"]
        .iter()
        .map(|path| stats.get(path).unwrap_or(0.0))
        .sum::<f64>();
    (accesses as u64, ctx)
}

/// The L1D accesses a 64-byte unit-stride load makes, after checking it
/// read every byte.
fn load_accesses(backend: BackendType, vector_mem_width: Option<usize>) -> u64 {
    let config = config(backend, vector_mem_width);
    let (with_load, mut ctx) = l1d_accesses(&config, true);
    let (without_load, _) = l1d_accesses(&config, false);
    let copied: Vec<u64> =
        (0..BYTES).map(|b| ctx.sim.probe_mem_load(PhysAddr::new(DEST + b), 1)).collect();
    assert_eq!(copied, (1..=BYTES).collect::<Vec<_>>(), "{backend:?}: the load read every byte");
    with_load - without_load
}

#[test]
fn a_unit_stride_load_reads_one_register_per_access_by_default() {
    for backend in BACKENDS {
        assert_eq!(
            load_accesses(backend, None),
            BYTES / 16,
            "{backend:?}: VLEN 128, 16-byte windows"
        );
    }
}

#[test]
fn the_vector_memory_width_sets_the_bytes_per_access() {
    for backend in BACKENDS {
        assert_eq!(load_accesses(backend, Some(8)), BYTES / 8, "{backend:?}: 8-byte windows");
        assert_eq!(load_accesses(backend, Some(64)), 1, "{backend:?}: one line");
    }
}

/// Loads 64 bytes from 32 bytes below the end of a 16 KiB RAM, trapping to
/// a spin loop.
fn load_past_the_end_of_ram() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(A0, 4).build(),                          // 0x00: RAM end
        i().addi(A0, A0, -32).build(),                     // 0x04
        i().auipc(T0, 0).build(),                          // 0x08
        i().addi(T0, T0, 0x20).build(),                    // 0x0c: the handler
        i().csrrw(0, MTVEC, T0).build(),                   // 0x10
        i().addi(A2, 0, BYTES as i32).build(),             // 0x14
        vsetvli_e8m4(A2),                                  // 0x18
        vle8(4, A0),                                       // 0x1c
        i().addi(DONE_REG as u32, 0, DONE as i32).build(), // 0x20
        i().jal(0, 0).build(),                             // 0x24
        i().jal(0, 0).build(),                             // 0x28: handler
    ]
}

#[test]
fn a_fault_inside_a_span_reports_the_first_element_that_faults() {
    for backend in BACKENDS {
        let mut config = config(backend, None);
        config.memory.ram_size = 0x4000;
        let mut ctx = TestContext::new_with_config(&config)
            .load_program(PROGRAM_BASE, &load_past_the_end_of_ram());

        ctx.run_until(20_000, |ctx| ctx.cpu().harts[0].csrs.mcause != 0).expect("the load trapped");

        let csrs = &ctx.cpu().harts[0].csrs;
        assert_eq!(
            (csrs.mcause, csrs.mtval, csrs.vstart),
            (LOAD_ACCESS_FAULT, PROGRAM_BASE + 0x4000, 32),
            "{backend:?}: element 32 is the first past the end"
        );
    }
}
