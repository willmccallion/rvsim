//! An `mret` discards every instruction fetched under the old privilege.
//!
//! Every instruction fetched while the hart was still in machine mode must
//! be squashed, even when the branch predictor already steered fetch to
//! the return address. Here the
//! BTB has learnt a jump to a supervisor virtual address; after the
//! handler's `mret` the wrong-path fetch takes that prediction under
//! machine-mode translation, where the address is not memory at all, and
//! the resulting fault must never reach commit.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::BackendKind;
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::isa::csr;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder, MRET};
use crate::tests::support::harness::TestContext;
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{A3, T0, T1, T2, T3, T4, T5, T6, X0};

const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const CODE_L1_PPN: u64 = 0x8_0101;
const TARGET_L1_PPN: u64 = 0x8_0102;
const CODE: u64 = 0x8020_0000;
const TARGET_PA: u64 = 0x8040_0000;
const TARGET_VA: u64 = 0x4000_0000;
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
        i().csrrw(X0, MTVEC, T2).build(),
        i().auipc(T3, 0).build(),
        i().addi(T3, T3, P * 4 - 16).build(),
        i().csrrw(X0, MEPC, T3).build(),
        i().lui(T4, 1).build(),
        i().addi(T4, T4, 0x7ff).build(),
        i().csrrc(X0, MSTATUS, T4).build(),
        i().addi(T4, X0, 1).build(),
        i().addi(T5, X0, 11).build(),
        i().sll(T4, T4, T5).build(),
        i().csrrs(X0, MSTATUS, T4).build(),
        i().addi(T6, X0, 0).build(),
        MRET,
        i().csrrs(T5, MCAUSE, X0).build(),
        i().addi(T4, X0, 1).build(),
        i().beq(T5, T4, (FAIL - 18) * 4).build(),
        i().addi(T6, T6, 1).build(),
        i().addi(T4, X0, 3).build(),
        i().bne(T6, T4, (RET - 21) * 4).build(),
        i().csrrs(T5, MEPC, X0).build(),
        i().addi(T5, T5, 4).build(),
        i().csrrw(X0, MEPC, T5).build(),
        MRET,
        i().jalr(X0, A3, 0).build(),
        i().addi(T1, X0, 1).build(),
        i().jal(X0, 0).build(),
    ];
    assert_eq!(code[M_HANDLER as usize], i().csrrs(T5, MCAUSE, X0).build());
    assert_eq!(code[RET as usize], MRET);
    assert_eq!(code[P as usize], i().jalr(X0, A3, 0).build());
    assert_eq!(code[FAIL as usize], i().addi(T1, X0, 1).build());
    code
}

fn target_code() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![ECALL, i().addi(T0, X0, 5).build(), i().jal(X0, 0).build()]
}

fn write_pte(ctx: &mut TestContext, table_ppn: u64, index: u64, pte: u64) {
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

/// `(t0, t1, t6)`: the target's result, the fault flag, and the ecall count.
fn run(backend: BackendKind, width: usize) -> (u64, u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (n, word) in machine_code().iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(CODE + (n as u64) * 4),
            u64::from(*word),
            ProbeWidth::B4,
        );
    }
    for (n, word) in target_code().iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(TARGET_PA + (n as u64) * 4),
            u64::from(*word),
            ProbeWidth::B4,
        );
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
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE);
    }
    ctx.sim.sync_arch_regs();

    ctx.run(5_000);
    (ctx.get_reg(T0), ctx.get_reg(T1), ctx.get_reg(T6))
}

fn a_predicted_fetch_across_mret_is_discarded(backend: BackendKind, width: usize) {
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
    a_predicted_fetch_across_mret_is_discarded(BackendKind::InOrder, 1);
    a_predicted_fetch_across_mret_is_discarded(BackendKind::InOrder, 4);
}

#[test]
fn o3_discards_machine_mode_fetches_after_mret() {
    a_predicted_fetch_across_mret_is_discarded(BackendKind::OutOfOrder, 1);
    a_predicted_fetch_across_mret_is_discarded(BackendKind::OutOfOrder, 4);
    a_predicted_fetch_across_mret_is_discarded(BackendKind::OutOfOrder, 10);
}
