//! A misaligned access that spills into the next page translates both pages.
//!
//! A fault on the second page is reported, and two pages that are not
//! physically adjacent leave the access to the misaligned trap handler
//! rather than reading the wrong bytes.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;
use rvsim_core::isa::csr;
use rvsim_core::isa::privileged::PrivilegeMode;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const CODE_L1_PPN: u64 = 0x8_0101;
const DATA_L1_PPN: u64 = 0x8_0102;
const DATA_L0_PPN: u64 = 0x8_0103;
const CODE: u64 = 0x8020_0000;
const HANDLER: u64 = CODE + 0x100;
const DATA_VA: u64 = 0x4000_0000;
const FIRST_PA: u64 = 0x8030_0000;
const ADJACENT_PA: u64 = FIRST_PA + 0x1000;
const FAR_PA: u64 = 0x8032_0000;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const LOAD_ADDRESS_MISALIGNED: u64 = 4;
const LOAD_PAGE_FAULT: u64 = 13;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
/// Eight bytes starting four bytes before the end of the first page.
const CROSSING_VA: u64 = DATA_VA + 0xFFC;

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

fn store_program(ctx: &mut TestContext, base: u64, program: &[u32]) {
    for (i, word) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(base + (i as u64) * 4), u64::from(*word), 4);
    }
}

/// `ld a1, 0(a0)` from `CROSSING_VA`, then a jump to self.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(A0, ((DATA_VA + 0x1000) >> 12) as i32).build(),
        i().addi(A0, A0, -4).build(),
        i().ld(A1, A0, 0).build(),
        i().jal(0, 0).build(),
    ]
}

/// The machine-mode handler records `mcause` and `mtval` and spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

struct Outcome {
    loaded: u64,
    mcause: u64,
    mtval: u64,
}

/// Runs the program in supervisor mode under Sv39 with the data's first
/// page at `FIRST_PA` and its second page at `second_pa` (unmapped when
/// `None`).
fn run(backend: BackendKind, second_pa: Option<u64>) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    store_program(&mut ctx, CODE, &program());
    store_program(&mut ctx, HANDLER, &handler());
    for (offset, byte) in (0u64..).zip(0x11u64..=0x18) {
        let pa = if offset < 4 {
            FIRST_PA + 0xFFC + offset
        } else {
            second_pa.unwrap_or(0) + offset - 4
        };
        if pa != 0 {
            ctx.sim.probe_mem_store(PhysAddr::new(pa), byte, 1);
        }
    }

    write_pte(&mut ctx, ROOT_PPN, (CODE >> 30) & 0x1ff, (CODE_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, CODE_L1_PPN, (CODE >> 21) & 0x1ff, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    write_pte(&mut ctx, ROOT_PPN, (DATA_VA >> 30) & 0x1ff, (DATA_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, DATA_L1_PPN, (DATA_VA >> 21) & 0x1ff, (DATA_L0_PPN << 10) | PTE_V);
    write_pte(
        &mut ctx,
        DATA_L0_PPN,
        (DATA_VA >> 12) & 0x1ff,
        ((FIRST_PA >> 12) << 10) | PTE_LEAF_RWX_AD,
    );
    if let Some(pa) = second_pa {
        write_pte(
            &mut ctx,
            DATA_L0_PPN,
            ((DATA_VA + 0x1000) >> 12) & 0x1ff,
            ((pa >> 12) << 10) | PTE_LEAF_RWX_AD,
        );
    }
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.mtvec = HANDLER;
        hart.privilege = PrivilegeMode::Supervisor;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(3_000);
    Outcome {
        loaded: ctx.get_reg(A1 as usize),
        mcause: ctx.get_reg(A2 as usize),
        mtval: ctx.get_reg(A3 as usize),
    }
}

fn check(backend: BackendKind) {
    let adjacent = run(backend, Some(ADJACENT_PA));
    assert_eq!(adjacent.mcause, 0, "{backend:?}: adjacent pages need no trap");
    assert_eq!(adjacent.loaded, 0x1817_1615_1413_1211, "{backend:?}: bytes from both pages");

    let far = run(backend, Some(FAR_PA));
    assert_eq!(far.mcause, LOAD_ADDRESS_MISALIGNED, "{backend:?}: split pages trap as misaligned");
    assert_eq!(far.mtval, CROSSING_VA, "{backend:?}: tval is the access address");

    let unmapped = run(backend, None);
    assert_eq!(
        unmapped.mcause, LOAD_PAGE_FAULT,
        "{backend:?}: the second page's fault is reported"
    );
    assert_eq!(unmapped.mtval, DATA_VA + 0x1000, "{backend:?}: tval names the faulting page");
}

#[test]
fn a_page_crossing_load_translates_both_pages_inorder() {
    check(BackendKind::InOrder);
}

#[test]
fn a_page_crossing_load_translates_both_pages_o3() {
    check(BackendKind::OutOfOrder);
}
