//! An `mret` discards every instruction fetched under the old privilege.
//!
//! Every instruction fetched while the hart was still in machine mode must
//! be squashed, even when the branch predictor already steered fetch to
//! the return address. Here the
//! BTB has learnt a jump to a supervisor virtual address; after the
//! handler's `mret` the wrong-path fetch takes that prediction under
//! machine-mode translation, where the address is not memory at all, and
//! the resulting fault must never reach commit.

use crate::common::builder::instruction::{ECALL, InstructionBuilder, MRET};
use crate::common::harness::TestContext;
use rvsim_core::arch::csr;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const CODE_L1_PPN: u64 = 0x8_0101;
const TARGET_L1_PPN: u64 = 0x8_0102;
const CODE: u64 = 0x8020_0000;
const TARGET_PA: u64 = 0x8040_0000;
const TARGET_VA: u64 = 0x4000_0000;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;
const T5: u32 = 30;
const T6: u32 = 31;
const A3: u32 = 13;
const MSTATUS: u32 = 0x300;
const MTVEC: u32 = 0x305;
const MEPC: u32 = 0x341;
const MCAUSE: u32 = 0x342;
const PTE_V: u64 = 1;
const PTE_LEAF_RWX_AD: u64 = 0b1100_1111;
const M_HANDLER: i32 = 16;
const RET: i32 = 25;
const P: i32 = 26;
const FAIL: i32 = 27;

fn machine_code() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let code = vec![
        i().lui(A3, 0x40000).build(),
        i().auipc(T2, 0).build(),
        i().addi(T2, T2, M_HANDLER * 4 - 4).build(),
        i().csrrw(0, MTVEC, T2).build(),
        i().auipc(T3, 0).build(),
        i().addi(T3, T3, P * 4 - 16).build(),
        i().csrrw(0, MEPC, T3).build(),
        i().lui(T4, 1).build(),
        i().addi(T4, T4, 0x7ff).build(),
        i().csrrc(0, MSTATUS, T4).build(),
        i().addi(T4, 0, 1).build(),
        i().addi(T5, 0, 11).build(),
        i().sll(T4, T4, T5).build(),
        i().csrrs(0, MSTATUS, T4).build(),
        i().addi(T6, 0, 0).build(),
        MRET,
        i().csrrs(T5, MCAUSE, 0).build(),
        i().addi(T4, 0, 1).build(),
        i().beq(T5, T4, (FAIL - 18) * 4).build(),
        i().addi(T6, T6, 1).build(),
        i().addi(T4, 0, 3).build(),
        i().bne(T6, T4, (RET - 21) * 4).build(),
        i().csrrs(T5, MEPC, 0).build(),
        i().addi(T5, T5, 4).build(),
        i().csrrw(0, MEPC, T5).build(),
        MRET,
        i().jalr(0, A3, 0).build(),
        i().addi(T1, 0, 1).build(),
        i().jal(0, 0).build(),
    ];
    assert_eq!(code[M_HANDLER as usize], i().csrrs(T5, MCAUSE, 0).build());
    assert_eq!(code[RET as usize], MRET);
    assert_eq!(code[P as usize], i().jalr(0, A3, 0).build());
    assert_eq!(code[FAIL as usize], i().addi(T1, 0, 1).build());
    code
}

fn target_code() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![ECALL, i().addi(T0, 0, 5).build(), i().jal(0, 0).build()]
}

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, 8);
}

/// `(t0, t1, t6)`: the target's result, the fault flag, and the ecall count.
fn run(backend: BackendType, width: usize) -> (u64, u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, word) in machine_code().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(CODE + (n as u64) * 4), u64::from(*word), 4);
    }
    for (n, word) in target_code().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(TARGET_PA + (n as u64) * 4), u64::from(*word), 4);
    }
    write_pte(&mut ctx, ROOT_PPN, (CODE >> 30) & 0x1ff, (CODE_L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, CODE_L1_PPN, (CODE >> 21) & 0x1ff, ((CODE >> 12) << 10) | PTE_LEAF_RWX_AD);
    write_pte(&mut ctx, ROOT_PPN, (TARGET_VA >> 30) & 0x1ff, (TARGET_L1_PPN << 10) | PTE_V);
    write_pte(
        &mut ctx,
        TARGET_L1_PPN,
        (TARGET_VA >> 21) & 0x1ff,
        ((TARGET_PA >> 12) << 10) | PTE_LEAF_RWX_AD,
    );
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.pmp.set_addr(0, u64::MAX >> 10);
        hart.pmp.set_cfg(0, 0b0000_1111);
        hart.pc = CODE;
    }
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(5_000);
    (ctx.get_reg(T0 as usize), ctx.get_reg(T1 as usize), ctx.get_reg(T6 as usize))
}

fn a_predicted_fetch_across_mret_is_discarded(backend: BackendType, width: usize) {
    let (t0, fault, ecalls) = run(backend, width);
    assert_eq!(
        fault, 0,
        "{backend:?} w{width}: a machine-mode fetch of the supervisor target reached commit"
    );
    assert_eq!(
        ecalls, 3,
        "{backend:?} w{width}: the ecall re-executed until the handler skipped it"
    );
    assert_eq!(t0, 5, "{backend:?} w{width}: the supervisor code after the ecall ran");
}

#[test]
fn inorder_discards_machine_mode_fetches_after_mret() {
    a_predicted_fetch_across_mret_is_discarded(BackendType::InOrder, 1);
    a_predicted_fetch_across_mret_is_discarded(BackendType::InOrder, 4);
}

#[test]
fn o3_discards_machine_mode_fetches_after_mret() {
    a_predicted_fetch_across_mret_is_discarded(BackendType::OutOfOrder, 1);
    a_predicted_fetch_across_mret_is_discarded(BackendType::OutOfOrder, 4);
    a_predicted_fetch_across_mret_is_discarded(BackendType::OutOfOrder, 10);
}
