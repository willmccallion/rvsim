//! O3 Execute: single-instruction execution for the out-of-order backend.
//!
//! This module provides `execute_one()` which executes a single issued
//! instruction. It directly calls the shared hardware units (Alu, Fpu, BRU)
//! and handles all instruction types: ALU, FP, branch, jump, CSR,
//! MRET/SRET, WFI, SFENCE.VMA, ECALL, FENCE.I, and trap propagation.
//!
//! This is independent from the in-order execute — both call the same
//! hardware units but are structured differently.

use crate::common::SfenceVmaInfo;
use crate::common::error::{ExceptionStage, Trap};
use crate::core::pipeline::backend::shared::vector_config::set_vector_config;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{BpOutcome, CsrUpdate, Rob};
use crate::core::pipeline::signals::{
    AluOp, ControlFlow, CsrOp, OpASrc, OpBSrc, SystemOp, VectorOp,
};
use crate::core::pipeline::squash::{BranchRepair, Redirect, SquashCause};
use crate::core::units::alu::Alu;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::fpu::Fpu;
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use crate::isa::rv64i::{funct3, opcodes};
use crate::sim::CoreCtx;
use crate::trace_branch;
use crate::trace_csr;
use crate::trace_execute;
use crate::trace_trap;

const FUNCT3_SHIFT: u32 = 12;

/// The instruction after `id` in program order.
const fn next_pc(id: &RenameIssueEntry) -> u64 {
    id.pc.wrapping_add(id.inst_size.as_u64())
}
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;

/// Execute a single instruction for the O3 backend.
///
/// Returns the result and, when the instruction squashes what follows it
/// (misprediction, CSR write, MRET/SRET, FENCE.I, a fault), the
/// [`Redirect`] the engine takes after the redirect latency.
pub fn execute_one(
    state: &mut CoreCtx<'_>,
    id: RenameIssueEntry,
    rob: &mut Rob,
) -> (ExMem1Entry, Option<Redirect>) {
    if let Some(trap) = id.trap.clone() {
        trace_execute!(state.config.general.trace_instructions;
            rob_tag         = id.rob_tag.0,
            pc              = %crate::trace::Hex(id.pc),
            inst            = %crate::trace::Hex32(id.inst),
            trap            = ?trap,
            stage           = ?id.exception_stage,
            "EX: trap propagated from earlier stage"
        );
        rob.fault(id.rob_tag, trap, id.exception_stage.unwrap_or(ExceptionStage::Execute));
        let result = ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: 0,
            store_data: 0,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };
        return (result, Some(Redirect::to(next_pc(&id), SquashCause::System)));
    }

    if state.check_execute_trigger(id.pc) {
        let result = ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: 0,
            store_data: 0,
            ctrl: id.ctrl,
            trap: Some(crate::common::Trap::Breakpoint(id.pc)),
            exception_stage: Some(crate::common::error::ExceptionStage::Execute),
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };
        return (result, None);
    }

    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::trace::Hex(id.pc),
        inst     = %crate::trace::Hex32(id.inst),
        rd       = id.rd.as_usize(),
        rd_phys  = id.rd_phys.0,
        rs1      = id.rs1.as_usize(),
        rs1_phys = id.rs1_phys.0,
        rv1      = %crate::trace::Hex(id.rv1),
        rs2      = id.rs2.as_usize(),
        rs2_phys = id.rs2_phys.0,
        rv2      = %crate::trace::Hex(id.rv2),
        imm      = id.imm,
        a_src    = ?id.ctrl.a_src,
        b_src    = ?id.ctrl.b_src,
        alu_op   = ?id.ctrl.alu,
        is_rv32  = id.ctrl.is_rv32,
        is_fp    = id.ctrl.fp_reg_write,
        "EX: begin"
    );

    let fwd_a = id.rv1;
    let fwd_b = id.rv2;
    let fwd_c = id.rv3;
    let store_data = fwd_b;

    let op_a = match id.ctrl.a_src {
        OpASrc::Reg1 => fwd_a,
        OpASrc::Pc => id.pc,
        OpASrc::Zero => 0,
    };
    let op_b = match id.ctrl.b_src {
        OpBSrc::Reg2 => fwd_b,
        OpBSrc::Imm => id.imm as u64,
        OpBSrc::Zero => 0,
    };
    let op_c = fwd_c;

    // Intercept vector ops before the system-instruction handler.
    if id.ctrl.vec_op != VectorOp::None {
        if id.ctrl.vec_op.is_config() {
            let vl = set_vector_config(state, &id, fwd_a, fwd_b, rob);
            let result = ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: vl,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            };
            return (result, None);
        }

        // Non-vsetvl vector ops are executed in O3Engine::tick() where VecPrfView is available.
        let result = ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: op_a,
            store_data: fwd_b,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };
        return (result, None);
    }

    // I-cache flush deferred to commit so prior stores are visible before refill.
    if id.ctrl.system_op == SystemOp::FenceI {
        let next_pc = id.pc.wrapping_add(id.inst_size.as_u64());
        trace_execute!(state.config.general.trace_instructions;
            rob_tag = id.rob_tag.0,
            pc      = %crate::trace::Hex(id.pc),
            next_pc = %crate::trace::Hex(next_pc),
            "EX: FENCE.I — pipeline flush, I-cache invalidation deferred to commit"
        );

        let result = ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: 0,
            store_data: 0,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };
        return (result, Some(Redirect::to(next_pc, SquashCause::System)));
    }

    // FENCE is a NOP at execute — handled at commit only.
    if !matches!(id.ctrl.system_op, SystemOp::None | SystemOp::Fence) {
        return execute_system(state, id, rob, fwd_a, store_data);
    }

    // When mstatus.FS == OFF, all FP instructions trap as illegal.
    {
        let fs = (state.hart.csrs.mstatus & crate::core::arch::csr::MSTATUS_FS) >> 13;
        let is_fp = id.ctrl.fp_reg_write || id.ctrl.rs1_fp || id.ctrl.rs2_fp || id.ctrl.rs3_fp;
        if fs == 0 && is_fp {
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            let result = ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            };
            return (result, Some(Redirect::to(next_pc(&id), SquashCause::System)));
        }
    }

    // Resolve FP rounding mode: instruction-specified or dynamic from fcsr.frm.
    let fp_rm = id.ctrl.fp_rm.or_else(|| RoundingMode::from_bits(state.hart.csrs.frm as u8));
    let (alu_out, fp_flags) =
        compute_alu(id.ctrl.alu, op_a, op_b, op_c, id.ctrl.is_f16, id.ctrl.is_rv32, fp_rm);
    trace_execute!(state.config.general.trace_instructions;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::trace::Hex(id.pc),
        op_a     = %crate::trace::Hex(op_a),
        op_b     = %crate::trace::Hex(op_b),
        result   = %crate::trace::Hex(alu_out),
        fp_flags,
        "EX: ALU/FPU result"
    );

    let mut redirect = None;

    if id.ctrl.control_flow == ControlFlow::Branch {
        let taken = match (id.inst >> FUNCT3_SHIFT) & FUNCT3_MASK {
            funct3::BEQ => op_a == op_b,
            funct3::BNE => op_a != op_b,
            funct3::BLT => (op_a as i64) < (op_b as i64),
            funct3::BGE => (op_a as i64) >= (op_b as i64),
            funct3::BLTU => op_a < op_b,
            funct3::BGEU => op_a >= op_b,
            _ => false,
        };
        let actual_target = id.pc.wrapping_add(id.imm as u64);
        let fallthrough = id.pc.wrapping_add(id.inst_size.as_u64());

        let predicted_target = if id.pred_taken { id.pred_target } else { fallthrough };
        let actual_next_pc = if taken { actual_target } else { fallthrough };

        let mispredicted = predicted_target != actual_next_pc;

        // Defer BP update to commit so wrong-path data doesn't pollute the tables.
        rob.set_bp_update(
            id.rob_tag,
            id.pc,
            BpOutcome { taken, mispredicted },
            if taken { Some(actual_target) } else { None },
            id.ghr_snapshot,
        );

        trace_branch!(state.config.general.trace_instructions;
            event          = "resolve",
            pc             = %crate::trace::Hex(id.pc),
            rob_tag        = id.rob_tag.0,
            pred_taken     = id.pred_taken,
            pred_target    = %crate::trace::Hex(predicted_target),
            actual_taken   = taken,
            actual_target  = %crate::trace::Hex(actual_next_pc),
            mispredicted,
            "EX: branch resolved"
        );
        if mispredicted {
            state.shared.stats.counter(state.core.stat_paths.bp.spec_mispredicts).inc();
            let repair =
                BranchRepair { pc: id.pc, taken, ghr: id.ghr_snapshot, ras: id.ras_snapshot };
            redirect = Some(Redirect::mispredict(actual_next_pc, repair));
        } else {
            state.shared.stats.counter(state.core.stat_paths.bp.spec_hits).inc();
        }
    }

    if id.ctrl.control_flow == ControlFlow::Jump {
        use crate::common::constants::OPCODE_MASK;
        let is_jalr = (id.inst & OPCODE_MASK) == opcodes::OP_JALR;
        let rd_link = id.rd == abi::REG_RA || id.rd == abi::REG_T0;
        let rs1_link = is_jalr && (id.rs1 == abi::REG_RA || id.rs1 == abi::REG_T0);

        let actual_target = if is_jalr {
            (fwd_a.wrapping_add(id.imm as u64)) & JALR_ALIGNMENT_MASK
        } else {
            id.pc.wrapping_add(id.imm as u64)
        };

        let predicted_target =
            if id.pred_taken { id.pred_target } else { id.pc.wrapping_add(id.inst_size.as_u64()) };

        let mispredicted = actual_target != predicted_target;

        // Record target for committed_next_pc but skip bp_update: jumps don't train direction.
        rob.set_bp_target(id.rob_tag, actual_target);

        if is_jalr {
            state.core.branch_predictor.update_btb(id.pc, actual_target);
        }

        trace_branch!(state.config.general.trace_instructions;
            event          = "resolve",
            pc             = %crate::trace::Hex(id.pc),
            rob_tag        = id.rob_tag.0,
            bp_type        = if rs1_link && !rd_link { "JALR/RAS" } else if rd_link { "JAL/call" } else { "JAL/JALR" },
            pred_taken     = id.pred_taken,
            pred_target    = %crate::trace::Hex(predicted_target),
            actual_taken   = true,
            actual_target  = %crate::trace::Hex(actual_target),
            mispredicted,
            "EX: jump resolved"
        );
        if mispredicted {
            state.shared.stats.counter(state.core.stat_paths.bp.spec_mispredicts).inc();
            let repair =
                BranchRepair { pc: id.pc, taken: true, ghr: id.ghr_snapshot, ras: id.ras_snapshot };
            redirect = Some(Redirect::mispredict(actual_target, repair));
        } else {
            state.shared.stats.counter(state.core.stat_paths.bp.spec_hits).inc();
        }
    }

    let result = ExMem1Entry {
        rob_tag: id.rob_tag,
        pc: id.pc,
        inst: id.inst,
        inst_size: id.inst_size,
        rd: id.rd,
        alu: alu_out,
        store_data,
        ctrl: id.ctrl,
        trap: None,
        exception_stage: None,
        rd_phys: id.rd_phys,
        fp_flags,
        sfence_vma: None,
        vec_mem: None,
    };

    (result, redirect)
}

/// Handle system instructions (MRET, SRET, WFI, SFENCE.VMA, ECALL, CSR).
fn execute_system(
    state: &CoreCtx<'_>,
    id: RenameIssueEntry,
    rob: &mut Rob,
    fwd_a: u64,
    store_data: u64,
) -> (ExMem1Entry, Option<Redirect>) {
    let make_result =
        |alu: u64, ctrl: crate::core::pipeline::signals::ControlSignals| ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu,
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };

    if id.ctrl.system_op == SystemOp::Mret {
        if state.hart.privilege != crate::core::arch::mode::PrivilegeMode::Machine {
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (
                make_result(0, id.ctrl),
                Some(Redirect::to(next_pc(&id), SquashCause::System)),
            );
        }
        trace_trap!(state.config.general.trace_instructions;
            event       = "return",
            pc          = %crate::trace::Hex(id.pc),
            rob_tag     = id.rob_tag.0,
            insn        = "MRET",
            priv_mode   = ?state.hart.privilege,
            mepc        = %crate::trace::Hex(state.hart.csrs.mepc),
            mstatus     = %crate::trace::Hex(state.hart.csrs.mstatus),
            "EX: MRET queued (privilege restore deferred to commit)"
        );
        return (make_result(0, id.ctrl), Some(Redirect::to(next_pc(&id), SquashCause::System)));
    }

    if id.ctrl.system_op == SystemOp::Sret {
        if state.hart.privilege == crate::core::arch::mode::PrivilegeMode::User {
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (
                make_result(0, id.ctrl),
                Some(Redirect::to(next_pc(&id), SquashCause::System)),
            );
        }
        let tsr = (state.hart.csrs.mstatus >> 22) & 1;
        if state.hart.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tsr != 0 {
            trace_trap!(state.trace_trap_enabled(&Trap::IllegalInstruction(id.inst));
                event   = "illegal",
                pc      = %crate::trace::Hex(id.pc),
                rob_tag = id.rob_tag.0,
                insn    = "SRET",
                reason  = "TSR=1 in S-mode",
                "EX: SRET -> IllegalInstruction (TSR)"
            );
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (
                make_result(0, id.ctrl),
                Some(Redirect::to(next_pc(&id), SquashCause::System)),
            );
        }
        trace_trap!(state.config.general.trace_instructions;
            event     = "return",
            pc        = %crate::trace::Hex(id.pc),
            rob_tag   = id.rob_tag.0,
            insn      = "SRET",
            priv_mode = ?state.hart.privilege,
            sepc      = %crate::trace::Hex(state.hart.csrs.sepc),
            mstatus   = %crate::trace::Hex(state.hart.csrs.mstatus),
            "EX: SRET queued (privilege restore deferred to commit)"
        );
        return (make_result(0, id.ctrl), Some(Redirect::to(next_pc(&id), SquashCause::System)));
    }

    if id.ctrl.system_op == SystemOp::Wfi {
        let tw = (state.hart.csrs.mstatus >> 21) & 1;
        if state.hart.privilege == crate::core::arch::mode::PrivilegeMode::User
            || (state.hart.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor
                && tw != 0)
        {
            trace_trap!(state.trace_trap_enabled(&Trap::IllegalInstruction(id.inst));
                event   = "illegal",
                pc      = %crate::trace::Hex(id.pc),
                insn    = "WFI",
                reason  = "U-mode or TW=1",
                "EX: WFI -> IllegalInstruction"
            );
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
        }
        return (make_result(0, id.ctrl), Some(Redirect::to(next_pc(&id), SquashCause::System)));
    }

    // SFENCE.VMA: do nothing at execute. Operands flow to commit which drains
    // the store buffer first, then performs the TLB flush and pipeline squash.
    if id.ctrl.system_op == SystemOp::SfenceVma {
        let tvm = (state.hart.csrs.mstatus >> 20) & 1;
        if state.hart.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tvm != 0 {
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (
                ExMem1Entry {
                    rob_tag: id.rob_tag,
                    pc: id.pc,
                    inst: id.inst,
                    inst_size: id.inst_size,
                    rd: id.rd,
                    alu: 0,
                    store_data,
                    ctrl: id.ctrl,
                    trap: None,
                    exception_stage: None,
                    rd_phys: id.rd_phys,
                    fp_flags: 0,
                    sfence_vma: None,
                    vec_mem: None,
                },
                Some(Redirect::to(next_pc(&id), SquashCause::System)),
            );
        }

        return (
            ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: Some(SfenceVmaInfo {
                    rs1_idx: id.rs1,
                    rs2_idx: id.rs2,
                    rs1_val: fwd_a,
                    rs2_val: store_data,
                }),
                vec_mem: None,
            },
            None,
        );
    }

    // CBO instructions (Zicboz / Zicbom): all side effects, the privilege
    // gate, and the address translation happen at commit so faults route
    // through the standard commit-time trap path and stale PTEs after a
    // pending PTE store are picked up correctly. Execute just forwards rs1.
    if matches!(
        id.ctrl.system_op,
        SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush
    ) {
        return (
            make_result(fwd_a, id.ctrl),
            Some(Redirect::to(next_pc(&id), SquashCause::System)),
        );
    }

    if id.inst == sys_ops::ECALL {
        use crate::core::arch::mode::PrivilegeMode;
        let trap = match state.hart.privilege {
            PrivilegeMode::User => Trap::EnvironmentCallFromUMode,
            PrivilegeMode::Supervisor => Trap::EnvironmentCallFromSMode,
            PrivilegeMode::Machine => Trap::EnvironmentCallFromMMode,
        };
        trace_trap!(state.trace_trap_enabled(&trap);
            event     = "take",
            pc        = %crate::trace::Hex(id.pc),
            rob_tag   = id.rob_tag.0,
            cause     = "ECALL",
            priv_mode = ?state.hart.privilege,
            a7        = %crate::trace::Hex(state.hart.regs.read(crate::isa::abi::REG_A7)),
            a0        = %crate::trace::Hex(state.hart.regs.read(crate::isa::abi::REG_A0)),
            "EX: ECALL"
        );
        rob.fault(id.rob_tag, trap, ExceptionStage::Execute);
        return (make_result(0, id.ctrl), Some(Redirect::to(next_pc(&id), SquashCause::System)));
    }

    if id.ctrl.csr_op != CsrOp::None {
        return execute_csr(state, id, rob, fwd_a, store_data);
    }

    (make_result(0, id.ctrl), Some(Redirect::to(next_pc(&id), SquashCause::System)))
}

/// Handle CSR operations.
#[allow(clippy::needless_pass_by_value)]
fn execute_csr(
    state: &CoreCtx<'_>,
    id: RenameIssueEntry,
    rob: &mut Rob,
    fwd_a: u64,
    store_data: u64,
) -> (ExMem1Entry, Option<Redirect>) {
    if id.ctrl.csr_addr == crate::core::arch::csr::SATP
        && state.hart.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor
        && ((state.hart.csrs.mstatus >> 20) & 1) != 0
    {
        rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
        return (
            ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            },
            Some(Redirect::to(next_pc(&id), SquashCause::System)),
        );
    }

    // mcounteren / scounteren check for CYCLE/TIME/INSTRET.
    {
        use crate::core::arch::csr as csr_addrs;
        use crate::core::arch::mode::PrivilegeMode;
        let counter_bit = if id.ctrl.csr_addr == csr_addrs::CYCLE {
            Some(0)
        } else if id.ctrl.csr_addr == csr_addrs::TIME {
            Some(1)
        } else if id.ctrl.csr_addr == csr_addrs::INSTRET {
            Some(2)
        } else {
            None
        };
        if let Some(bit) = counter_bit {
            let mask = 1u64 << bit;
            let denied = match state.hart.privilege {
                PrivilegeMode::Supervisor => (state.hart.csrs.mcounteren & mask) == 0,
                PrivilegeMode::User => {
                    (state.hart.csrs.mcounteren & mask) == 0
                        || (state.hart.csrs.scounteren & mask) == 0
                }
                PrivilegeMode::Machine => false,
            };
            if denied {
                rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
                return (
                    ExMem1Entry {
                        rob_tag: id.rob_tag,
                        pc: id.pc,
                        inst: id.inst,
                        inst_size: id.inst_size,
                        rd: id.rd,
                        alu: 0,
                        store_data: 0,
                        ctrl: id.ctrl,
                        trap: None,
                        exception_stage: None,
                        rd_phys: id.rd_phys,
                        fp_flags: 0,
                        sfence_vma: None,
                        vec_mem: None,
                    },
                    Some(Redirect::to(next_pc(&id), SquashCause::System)),
                );
            }
        }
    }

    if !state.is_valid_csr(id.ctrl.csr_addr) {
        rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
        return (
            ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            },
            Some(Redirect::to(next_pc(&id), SquashCause::System)),
        );
    }

    let csr_priv = id.ctrl.csr_addr.privilege_level() as u32;
    if (state.hart.privilege.to_u8() as u32) < csr_priv {
        rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
        return (
            ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: id.rd_phys,
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            },
            Some(Redirect::to(next_pc(&id), SquashCause::System)),
        );
    }

    let read_only = id.ctrl.csr_addr.is_read_only();
    if read_only {
        let would_write = match id.ctrl.csr_op {
            CsrOp::Rw | CsrOp::Rwi => true,
            CsrOp::Rs | CsrOp::Rc => !id.rs1.is_zero(),
            CsrOp::Rsi | CsrOp::Rci => (id.rs1.as_u8() & 0x1f) != 0,
            CsrOp::None => false,
        };
        if would_write {
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (
                ExMem1Entry {
                    rob_tag: id.rob_tag,
                    pc: id.pc,
                    inst: id.inst,
                    inst_size: id.inst_size,
                    rd: id.rd,
                    alu: 0,
                    store_data: 0,
                    ctrl: id.ctrl,
                    trap: None,
                    exception_stage: None,
                    rd_phys: id.rd_phys,
                    fp_flags: 0,
                    sfence_vma: None,
                    vec_mem: None,
                },
                Some(Redirect::to(next_pc(&id), SquashCause::System)),
            );
        }
    }

    let old = state.csr_read(id.ctrl.csr_addr);
    let base = state.csr_read_for_update(id.ctrl.csr_addr);
    let src = match id.ctrl.csr_op {
        CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => id.rs1.as_usize() as u64 & 0x1f,
        _ => fwd_a,
    };
    let new = match id.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => src,
        CsrOp::Rs | CsrOp::Rsi => base | src,
        CsrOp::Rc | CsrOp::Rci => base & !src,
        CsrOp::None => old,
    };

    trace_csr!(state.config.general.trace_instructions;
        op        = "write-deferred",
        pc        = %crate::trace::Hex(id.pc),
        rob_tag   = id.rob_tag.0,
        csr_addr  = %crate::trace::Hex32(id.ctrl.csr_addr.as_u32()),
        csr_op    = ?id.ctrl.csr_op,
        old_val   = %crate::trace::Hex(old),
        new_val   = %crate::trace::Hex(new),
        rd        = id.rd.as_usize(),
        "EX: CSR deferred write queued in ROB"
    );
    // CSRRS/CSRRC with rs1=x0 and CSRRSI/CSRRCI with uimm=0 must not write (spec §2.8).
    let would_write = match id.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => true,
        CsrOp::Rs | CsrOp::Rc => !id.rs1.is_zero(),
        CsrOp::Rsi | CsrOp::Rci => (id.rs1.as_u8() & 0x1f) != 0,
        CsrOp::None => false,
    };
    if would_write {
        rob.set_csr_update(
            id.rob_tag,
            CsrUpdate { addr: id.ctrl.csr_addr, old_val: old, new_val: new, applied: false },
        );
    }

    // Only CSR writes need a flush; pure reads stay serialized at issue time.
    let redirect = would_write
        .then(|| Redirect::to(id.pc.wrapping_add(id.inst_size.as_u64()), SquashCause::System));

    (
        ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: old,
            store_data,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        },
        redirect,
    )
}

/// Compute ALU/FPU result and return (result, `fp_flags`).
fn compute_alu(
    alu_op: AluOp,
    op_a: u64,
    op_b: u64,
    op_c: u64,
    is_f16: bool,
    is_rv32: bool,
    fp_rm: Option<RoundingMode>,
) -> (u64, u8) {
    // FP conversions go through the host FPU so we capture INEXACT / OVERFLOW etc.
    match alu_op {
        AluOp::FCvtSW
        | AluOp::FCvtSL
        | AluOp::FCvtSWU
        | AluOp::FCvtSLU
        | AluOp::FCvtSD
        | AluOp::FCvtDS
        | AluOp::FCvtSH
        | AluOp::FCvtDH
            if !is_f16 =>
        {
            use crate::core::units::fpu::half::{f16_to_f32, unbox_f16};
            use crate::core::units::fpu::nan_handling::{box_f32_canon, unbox_f32};
            use crate::core::units::fpu::{
                clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode,
                set_host_round_mode,
            };
            let rm = fp_rm.unwrap_or(RoundingMode::Rne);
            let saved = set_host_round_mode(rm);
            clear_host_fp_flags();
            let val = std::hint::black_box(match alu_op {
                AluOp::FCvtSW => {
                    if is_rv32 {
                        Fpu::box_f32(std::hint::black_box(op_a as i32) as f32)
                    } else {
                        (std::hint::black_box(op_a as i32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSWU => {
                    if is_rv32 {
                        Fpu::box_f32(std::hint::black_box(op_a as u32) as f32)
                    } else {
                        (std::hint::black_box(op_a as u32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSL => {
                    if is_rv32 {
                        Fpu::box_f32(std::hint::black_box(op_a as i64) as f32)
                    } else {
                        (std::hint::black_box(op_a as i64) as f64).to_bits()
                    }
                }
                AluOp::FCvtSLU => {
                    if is_rv32 {
                        Fpu::box_f32(std::hint::black_box(op_a) as f32)
                    } else {
                        (std::hint::black_box(op_a) as f64).to_bits()
                    }
                }
                AluOp::FCvtSD => {
                    let val_d = f64::from_bits(op_a);
                    let val_s = std::hint::black_box(val_d) as f32;
                    box_f32_canon(val_s)
                }
                AluOp::FCvtDS => {
                    use crate::core::units::fpu::nan_handling::canonicalize_f64_bits;
                    let val_s = unbox_f32(op_a);
                    let val_d = std::hint::black_box(val_s) as f64;
                    canonicalize_f64_bits(val_d)
                }
                AluOp::FCvtSH => {
                    let val_s = f16_to_f32(unbox_f16(op_a));
                    box_f32_canon(val_s)
                }
                AluOp::FCvtDH => {
                    use crate::core::units::fpu::nan_handling::canonicalize_f64_bits;
                    let val_s = f16_to_f32(unbox_f16(op_a));
                    canonicalize_f64_bits(std::hint::black_box(val_s) as f64)
                }
                _ => unreachable!(),
            });
            let fp_flags = read_host_fp_flags();
            restore_host_round_mode(saved);
            return (val, fp_flags.bits());
        }
        AluOp::FMvToF => {
            let val = if is_f16 {
                use crate::core::units::fpu::half::box_f16;
                box_f16(op_a as u16)
            } else if is_rv32 {
                Fpu::box_f32(f32::from_bits(op_a as u32))
            } else {
                op_a
            };
            return (val, 0);
        }
        _ => {}
    }

    let is_fp_op = matches!(
        alu_op,
        AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMul
            | AluOp::FDiv
            | AluOp::FSqrt
            | AluOp::FMin
            | AluOp::FMax
            | AluOp::FMAdd
            | AluOp::FMSub
            | AluOp::FNMAdd
            | AluOp::FNMSub
            | AluOp::FSgnJ
            | AluOp::FSgnJN
            | AluOp::FSgnJX
            | AluOp::FEq
            | AluOp::FLt
            | AluOp::FLe
            | AluOp::FClass
            | AluOp::FCvtWS
            | AluOp::FCvtWUS
            | AluOp::FCvtLS
            | AluOp::FCvtLUS
            | AluOp::FCvtSW
            | AluOp::FCvtSWU
            | AluOp::FCvtSL
            | AluOp::FCvtSLU
            | AluOp::FCvtSD
            | AluOp::FCvtDS
            | AluOp::FCvtSH
            | AluOp::FCvtHS
            | AluOp::FCvtDH
            | AluOp::FCvtHD
            | AluOp::FMvToX
    );

    if is_fp_op {
        let rm = fp_rm.unwrap_or(RoundingMode::Rne);
        let (result, fp_flags) =
            Fpu::execute_full_rm(alu_op, op_a, op_b, op_c, is_f16, is_rv32, rm);
        (result, fp_flags.bits())
    } else {
        (Alu::execute(alu_op, op_a, op_b, op_c, is_rv32), 0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx};
    use crate::config::Config;
    use crate::core::pipeline::signals::ControlSignals;
    use crate::core::units::bru::Ghr;
    use crate::core::units::bru::RasSnapshot;

    #[test]
    fn test_execute_one_normal() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 10,
            rv2: 20,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert!(redirect.is_none());
        assert_eq!(result.alu, 10); // rv1 (10) + 0
        assert_eq!(result.rob_tag, tag);
    }

    #[test]
    fn test_execute_trap_propagation() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: Some(Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Decode),
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert!(redirect.is_some());
        let entry = rob.find_entry(tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
        assert!(entry.trap.is_some());
    }

    #[test]
    fn test_execute_fence_i() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let ctrl = ControlSignals { system_op: SystemOp::FenceI, ..Default::default() };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(0),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(0x1004));
    }

    #[test]
    fn test_execute_fp_trap_when_fs_zero() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        state.hart.csrs.mstatus &= !crate::core::arch::csr::MSTATUS_FS; // Clear FS bits

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let ctrl = ControlSignals {
            fp_reg_write: true, // Make it an FP instruction
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert!(redirect.is_some());
        let entry = rob.find_entry(tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
    }

    #[test]
    fn test_execute_branch_misprediction() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let ctrl = ControlSignals {
            control_flow: ControlFlow::Branch,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: (0 << 12),
            inst_size: InstSize::Standard, // BEQ (funct3 = 0)
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(0),
            imm: 8,
            rv1: 10,
            rv2: 10,
            rv3: 0, // rv1 == rv2, so taken
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0, // Predicted NOT taken
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert_eq!(redirect.map(|r| r.target), Some(0x1008));
        assert!(redirect.is_some_and(|r| r.repair.is_some_and(|repair| repair.taken)));
        let entry = rob.find_entry(tag).unwrap();
        assert!(entry.bp_outcome.mispredicted);
    }

    #[test]
    fn test_execute_jump_jalr() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);
        let mut rob = Rob::new(4);

        let tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let ctrl = ControlSignals { control_flow: ControlFlow::Jump, ..Default::default() };

        let issue = RenameIssueEntry {
            rob_tag: tag,
            pc: 0x1000,
            inst: crate::isa::rv64i::opcodes::OP_JALR,
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0x15,
            rv1: 0x2000,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: crate::core::pipeline::prf::PhysReg(0),
            rs2_phys: crate::core::pipeline::prf::PhysReg(0),
            rs3_phys: crate::core::pipeline::prf::PhysReg(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            ctrl,
            trap: None,
            exception_stage: None,
            pred_taken: true,
            pred_target: 0, // Predicted incorrectly
            ghr_snapshot: Ghr::default(),
            ras_snapshot: RasSnapshot::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        };

        let (_result, redirect) = execute_one(&mut state, issue, &mut rob);
        assert!(redirect.is_some());

        let expected_target = (0x2000 + 0x15) & !1;
        assert_eq!(redirect.map(|r| r.target), Some(expected_target));
    }
}
