//! End-to-end tests for the Zicboz / Zicbom CBO instructions.

use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use rvsim_core::isa::encoding::zicboz::{
    CBO_CLEAN_IMM, CBO_FLUSH_IMM, CBO_INVAL_IMM, CBO_ZERO_IMM, CBOZ_BLOCK_SIZE,
};

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const NOP: u32 = 0x0000_0013;
/// `jal x0, 0` — branch-to-self, used as the "park here" tail of test
/// programs so PC doesn't run off into uninitialized memory and trap.
const JAL_SELF: u32 = 0x0000_006F;
const X10: u32 = 10;

/// Encode a CBO instruction. I-type with the variant-specific 12-bit imm,
/// rd=x0, funct3=CBO, opcode=MISC-MEM.
fn cbo(imm: i64, rs1: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | ((rs1 & 0x1F) << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM
}

fn cbo_zero(rs1: u32) -> u32 {
    cbo(CBO_ZERO_IMM, rs1)
}

fn cbo_inval(rs1: u32) -> u32 {
    cbo(CBO_INVAL_IMM, rs1)
}

fn cbo_clean(rs1: u32) -> u32 {
    cbo(CBO_CLEAN_IMM, rs1)
}

fn cbo_flush(rs1: u32) -> u32 {
    cbo(CBO_FLUSH_IMM, rs1)
}

fn fill_pattern(ctx: &mut TestContext, base: u64, len: u64) {
    for off in (0..len).step_by(8) {
        let val = 0xDEAD_BEEF_CAFE_F00Du64.wrapping_add(off);
        ctx.sim.probe_mem_store(PhysAddr::new(base + off), val, 8);
    }
}

fn read_u64(ctx: &mut TestContext, addr: u64) -> u64 {
    ctx.sim.probe_mem_load(PhysAddr::new(addr), 8)
}

/// Run `<inst> x10; jal x0, 0` after seeding x10 with `addr_in_x10`.
///
/// `mtvec` is pointed at the jal-self so a trap from the CBO op parks
/// at the self-loop instead of fetching from PC=0. PC reaches the loop
/// either through fall-through or trap redirection, so `mcause` only
/// reflects the CBO op (no secondary fault from running off the program).
fn run_cbo(ctx: &mut TestContext, inst: u32, addr_in_x10: u64) {
    let park_offset = 4u64;
    ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE), u64::from(inst), 4);
    ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + park_offset), u64::from(JAL_SELF), 4);
    // A few NOPs after the jal so any in-flight speculative fetch has
    // valid bytes to decode before the redirect lands.
    for i in 2..16 {
        ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + i * 4), u64::from(NOP), 4);
    }
    ctx.set_reg(X10 as usize, addr_in_x10);
    ctx.sim.set_pc(0, RAM_BASE);
    ctx.cpu_mut().harts[0].csrs.mtvec = RAM_BASE + park_offset;
    ctx.run(1000);
}

fn run_cbo_zero(ctx: &mut TestContext, addr_in_x10: u64) {
    run_cbo(ctx, cbo_zero(X10), addr_in_x10);
}

/// 64-byte aligned address: cbo.zero zeros exactly that block, leaving the
/// adjacent block untouched.
#[test]
fn cbo_zero_aligned_zeroes_one_block_only() {
    let data_addr = RAM_BASE + 0x1000;
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, data_addr, 2 * CBOZ_BLOCK_SIZE);

    run_cbo_zero(&mut ctx, data_addr);

    for off in (0..CBOZ_BLOCK_SIZE).step_by(8) {
        assert_eq!(read_u64(&mut ctx, data_addr + off), 0, "block must be zeroed at +{off}");
    }
    for off in (CBOZ_BLOCK_SIZE..2 * CBOZ_BLOCK_SIZE).step_by(8) {
        let expected = 0xDEAD_BEEF_CAFE_F00Du64.wrapping_add(off);
        assert_eq!(
            read_u64(&mut ctx, data_addr + off),
            expected,
            "neighbour block must be untouched at +{off}"
        );
    }
}

/// Spec: the low bits of rs1 below `CBOZ_BLOCK_SIZE` are ignored. Passing
/// an unaligned address still zeros the block at `rs1 & !(BLOCK_SIZE - 1)`,
/// not the bytes from rs1 onward.
#[test]
fn cbo_zero_ignores_low_address_bits() {
    let block_addr = RAM_BASE + 0x1000;
    let unaligned = block_addr + 17; // mid-block
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, block_addr, 2 * CBOZ_BLOCK_SIZE);

    run_cbo_zero(&mut ctx, unaligned);

    for off in (0..CBOZ_BLOCK_SIZE).step_by(8) {
        assert_eq!(
            read_u64(&mut ctx, block_addr + off),
            0,
            "block at rs1 & !63 must be zeroed at +{off}"
        );
    }
    for off in (CBOZ_BLOCK_SIZE..2 * CBOZ_BLOCK_SIZE).step_by(8) {
        let expected = 0xDEAD_BEEF_CAFE_F00Du64.wrapping_add(off);
        assert_eq!(
            read_u64(&mut ctx, block_addr + off),
            expected,
            "next block must be untouched at +{off}"
        );
    }
}

/// Default M-mode access succeeds without setting menvcfg.CBZE.
#[test]
fn cbo_zero_in_machine_mode_ignores_menvcfg_cbze() {
    let data_addr = RAM_BASE + 0x1000;
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    assert_eq!(ctx.cpu().harts[0].csrs.menvcfg & rvsim_core::core::arch::csr::MENVCFG_CBZE, 0);
    fill_pattern(&mut ctx, data_addr, CBOZ_BLOCK_SIZE);

    run_cbo_zero(&mut ctx, data_addr);

    for off in (0..CBOZ_BLOCK_SIZE).step_by(8) {
        assert_eq!(read_u64(&mut ctx, data_addr + off), 0);
    }
}

// U-mode CBO trap-kind assertions live in the gate-helper unit tests under
// `tests/unit/core/arch/csr/cbo_gates.rs`. End-to-end verification through
// the U-mode fetch path requires PMP setup (otherwise a fetch
// access-fault fires before the CBO instruction even reaches execute),
// and that's larger plumbing than this polish commit.

// A "U-mode with both CBZE bits set succeeds" test isn't included here:
// without PMP entries configured, U-mode store always denies regardless
// of CBZE. The riscv-tests / chipsalliance suites cover the success path
// end-to-end via real M-mode firmware that programs PMP first.

// ── Zicbom (cbo.inval / cbo.clean / cbo.flush) ──────────────────────────────

/// In M-mode, all three Zicbom ops succeed (CBCFE/CBIE not consulted).
/// We can't directly observe the cache from a black-box test, but we can
/// confirm no trap fired and that RAM still holds the original pattern
/// (cbo.inval/flush don't corrupt RAM in this simulator because all
/// stores are committed to RAM at retire time).
#[test]
fn cbo_inval_in_machine_mode_does_not_trap() {
    let data_addr = RAM_BASE + 0x1000;
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, data_addr, CBOZ_BLOCK_SIZE);

    run_cbo(&mut ctx, cbo_inval(X10), data_addr);

    assert_eq!(ctx.cpu().harts[0].csrs.mcause, 0);
    // Pattern survives because RAM is the source of truth in this simulator.
    let expected = 0xDEAD_BEEF_CAFE_F00Du64;
    assert_eq!(read_u64(&mut ctx, data_addr), expected);
}

#[test]
fn cbo_clean_in_machine_mode_does_not_trap() {
    let data_addr = RAM_BASE + 0x1000;
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, data_addr, CBOZ_BLOCK_SIZE);

    run_cbo(&mut ctx, cbo_clean(X10), data_addr);

    assert_eq!(ctx.cpu().harts[0].csrs.mcause, 0);
}

#[test]
fn cbo_flush_in_machine_mode_does_not_trap() {
    let data_addr = RAM_BASE + 0x1000;
    let mut ctx = TestContext::new_with_config(&Config::default()).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, data_addr, CBOZ_BLOCK_SIZE);

    run_cbo(&mut ctx, cbo_flush(X10), data_addr);

    assert_eq!(ctx.cpu().harts[0].csrs.mcause, 0);
}

// L1D-state-observation tests for cbo.inval / cbo.clean / cbo.flush live
// in the cache unit tests, where we can drive `CacheSim` directly without
// the harness's cache-bypass routing (TestContext sets `cache_base =
// u64::MAX` so loads skip the cache entirely).

/// A load that follows `cbo.zero` to the same block reads zeros: the CBO
/// takes effect at commit, so the load must not read the block before then.
fn check_load_after_cbo_zero_reads_zero(backend: rvsim_core::core::pipeline::engine::BackendType) {
    use crate::common::builder::instruction::InstructionBuilder;
    const X11: u32 = 11;
    const X12: u32 = 12;
    let data_addr = RAM_BASE + 0x1000;
    let mut config = Config::default();
    config.pipeline.backend = backend;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    fill_pattern(&mut ctx, data_addr, CBOZ_BLOCK_SIZE);
    let i = InstructionBuilder::new;
    let program =
        [cbo_zero(X10), i().ld(X11, X10, 8).build(), i().addi(X12, 0, 1).build(), JAL_SELF];
    for (n, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + 4 * n as u64), u64::from(*inst), 4);
    }
    ctx.set_reg(X10 as usize, data_addr);
    ctx.set_reg(X11 as usize, u64::MAX);
    ctx.sim.set_pc(0, RAM_BASE);
    ctx.cpu_mut().harts[0].csrs.mtvec = RAM_BASE + 12;
    ctx.sim.sync_arch_regs();

    let finished = ctx.run_until(2_000, |ctx| ctx.get_reg(X12 as usize) == 1);

    assert!(finished.is_some(), "{backend:?}: the program never finished");
    assert_eq!(ctx.cpu().harts[0].csrs.mcause, 0, "{backend:?}: no trap was taken");
    assert_eq!(ctx.get_reg(X11 as usize), 0, "{backend:?}: the load read the zeroed block");
}

#[test]
fn load_after_cbo_zero_reads_zero_in_order() {
    check_load_after_cbo_zero_reads_zero(rvsim_core::core::pipeline::engine::BackendType::InOrder);
}

#[test]
fn load_after_cbo_zero_reads_zero_out_of_order() {
    check_load_after_cbo_zero_reads_zero(
        rvsim_core::core::pipeline::engine::BackendType::OutOfOrder,
    );
}
