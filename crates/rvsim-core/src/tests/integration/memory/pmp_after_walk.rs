//! After a page-table walk the translated access is checked against PMP
//! over exactly its own bytes: a `cbo.zero` over 64 bytes, a word load
//! over four.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::{BackendKind, Config, Console};
use crate::isa::csr;
use crate::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use crate::isa::encoding::zicboz::{CBO_ZERO_IMM, CBOZ_BLOCK_SIZE};
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::pmp_entry;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const L0_PPN: u64 = 0x8_0102;
const CODE_VA: u64 = 0x4002_0000;
const CODE_PA: u64 = 0x8036_0000;
const DATA_VA: u64 = 0x4003_0000;
const DATA_PA: u64 = 0x8040_0000;
const PAGE: u64 = 0x1000;
const TRAP_PARK: u64 = RAM_BASE + 0x100;
const PTE_V: u64 = 1;
const PTE_RWX_AD: u64 = 0b1100_1111;
const PTE_RW_AD: u64 = 0b1100_0111;
const JAL_SELF: u32 = 0x0000_006F;
const X10: u32 = 10;
const X11: u32 = 11;
const FILL: u32 = 0xDEAD_BEEF;
const STORE_ACCESS_FAULT: u64 = 7;
const PMP_NA4_NONE: u8 = 0b10 << 3;
const PMP_NAPOT_RWX: u8 = (0b11 << 3) | 0b111;
/// The block `cbo.zero` targets, the data page's second.
const BLOCK: u64 = CBOZ_BLOCK_SIZE;
/// Four bytes inside that block, past its first eight, PMP denies.
const PROTECTED: u64 = BLOCK + 0x10;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

const fn cbo(imm: i64, rs1: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (rs1 << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM
}

/// `pmpaddr` of the naturally aligned power-of-two region at `base`.
const fn napot(base: u64, size: u64) -> u64 {
    (base >> 2) | ((size >> 3) - 1)
}

/// Runs `program` in supervisor mode from `CODE_VA`, with the data page
/// mapped and not yet in the TLB, after `pmp` sets up the PMP entries.
fn run(
    backend: BackendKind,
    program: &[u32],
    pmp: impl Fn(&mut crate::arch::pmp::Pmp),
) -> TestContext {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(CODE_PA + 4 * n as u64),
            u64::from(*inst),
            ProbeWidth::B4,
        );
    }
    ctx.sim.probe_mem_store(PhysAddr::new(TRAP_PARK), u64::from(JAL_SELF), ProbeWidth::B4);
    for off in (0..PAGE).step_by(4) {
        ctx.sim.probe_mem_store(PhysAddr::new(DATA_PA + off), u64::from(FILL), ProbeWidth::B4);
    }
    write_pte(&mut ctx, ROOT_PPN, CODE_VA, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, CODE_VA, 1, (L0_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L0_PPN, CODE_VA, 0, ((CODE_PA >> 12) << 10) | PTE_RWX_AD);
    write_pte(&mut ctx, L0_PPN, DATA_VA, 0, ((DATA_PA >> 12) << 10) | PTE_RW_AD);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.menvcfg = csr::MENVCFG_CBZE | csr::MENVCFG_CBCFE;
        hart.csrs.mtvec = TRAP_PARK;
        hart.privilege = PrivilegeMode::Supervisor;
        pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE_VA);
    }
    ctx.sim.sync_arch_regs();

    ctx.run(2_000);
    ctx
}

/// PMP denies supervisor access to four bytes inside the block and allows
/// everything else.
fn protect_four_bytes_of_the_block(pmp: &mut crate::arch::pmp::Pmp) {
    pmp.set_addr(pmp_entry(0), (DATA_PA + PROTECTED) >> 2);
    pmp.set_cfg(pmp_entry(0), PMP_NA4_NONE);
    pmp.set_addr(pmp_entry(1), u64::MAX >> 10);
    pmp.set_cfg(pmp_entry(1), PMP_NAPOT_RWX);
}

#[test]
fn cbo_zero_after_a_walk_cannot_zero_pmp_protected_bytes_of_its_block() {
    let i = InstructionBuilder::new;
    let program = [
        i().lui(X10, (DATA_VA >> 12) as i32).build(),
        i().addi(X10, X10, BLOCK as i32).build(),
        cbo(CBO_ZERO_IMM, X10),
        JAL_SELF,
    ];
    let results = [BackendKind::InOrder, BackendKind::OutOfOrder].map(|backend| {
        let mut ctx = run(backend, &program, protect_four_bytes_of_the_block);
        let protected = ctx.sim.probe_mem_load(PhysAddr::new(DATA_PA + PROTECTED), ProbeWidth::B4);
        (ctx.sim.state.harts[0].csrs.mcause, protected)
    });

    let faulted = (STORE_ACCESS_FAULT, u64::from(FILL));
    assert_eq!(results, [faulted; 2], "[InOrder, OutOfOrder]: (mcause, protected word)");
}

/// PMP grants the code page, the page tables and the data page, and
/// nothing past the data page.
fn grant_only_the_pages_in_use(pmp: &mut crate::arch::pmp::Pmp) {
    pmp.set_addr(pmp_entry(0), napot(DATA_PA, PAGE));
    pmp.set_cfg(pmp_entry(0), PMP_NAPOT_RWX);
    pmp.set_addr(pmp_entry(1), napot(CODE_PA, PAGE));
    pmp.set_cfg(pmp_entry(1), PMP_NAPOT_RWX);
    pmp.set_addr(pmp_entry(2), napot(ROOT_PPN << 12, 4 * PAGE));
    pmp.set_cfg(pmp_entry(2), PMP_NAPOT_RWX);
}

#[test]
fn word_load_at_the_end_of_a_pmp_region_after_a_walk_does_not_fault() {
    let i = InstructionBuilder::new;
    let program = [
        i().lui(X10, ((DATA_VA + PAGE) >> 12) as i32).build(),
        i().lw(X11, X10, -4).build(),
        JAL_SELF,
    ];
    let results = [BackendKind::InOrder, BackendKind::OutOfOrder].map(|backend| {
        let ctx = run(backend, &program, grant_only_the_pages_in_use);
        (ctx.sim.state.harts[0].csrs.mcause, ctx.get_reg(X11 as usize))
    });

    let loaded = (0, FILL as i32 as i64 as u64);
    assert_eq!(results, [loaded; 2], "[InOrder, OutOfOrder]: (mcause, x11)");
}
