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
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{BpOutcome, CsrUpdate, Rob};
use crate::core::pipeline::signals::{AluOp, ControlFlow, CsrOp, OpASrc, OpBSrc, SystemOp};
use crate::core::units::alu::Alu;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::fpu::Fpu;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use crate::isa::rv64i::{funct3, opcodes};
use crate::trace_branch;
use crate::trace_csr;
use crate::trace_execute;
use crate::trace_trap;

const FUNCT3_SHIFT: u32 = 12;
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;

/// Execute a single instruction for the O3 backend.
///
/// Returns `(ExMem1Entry, needs_flush)`. When `needs_flush` is true,
/// the engine must flush younger instructions (misprediction, CSR,
/// MRET/SRET, FENCE.I, etc.).
pub fn execute_one(cpu: &mut Cpu, id: RenameIssueEntry, rob: &mut Rob) -> (ExMem1Entry, bool) {
    // Propagate traps from earlier stages
    if let Some(trap) = id.trap.clone() {
        trace_execute!(cpu.trace;
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
        };
        return (result, true);
    }

    trace_execute!(cpu.trace;
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

    // FENCE.I: flush the pipeline so younger instructions are squashed.
    // The I-cache flush is deferred to COMMIT time (after store drain)
    // to ensure that all prior stores are visible in RAM before the
    // I-cache refills with the new data.
    if id.ctrl.system_op == SystemOp::FenceI {
        cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
        cpu.redirect_pending = true;
        trace_execute!(cpu.trace;
            rob_tag = id.rob_tag.0,
            pc      = %crate::trace::Hex(id.pc),
            next_pc = %crate::trace::Hex(cpu.pc),
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
        };
        return (result, true);
    }

    // System instructions (FENCE is a NOP at execute — handled at commit only)
    if !matches!(id.ctrl.system_op, SystemOp::None | SystemOp::Fence) {
        return execute_system(cpu, id, rob, fwd_a, store_data);
    }

    // When mstatus.FS == OFF, all FP instructions trap as illegal.
    {
        let fs = (cpu.csrs.mstatus & crate::core::arch::csr::MSTATUS_FS) >> 13;
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
            };
            return (result, true);
        }
    }

    // ALU / FPU execution
    let (alu_out, fp_flags) = compute_alu(id.ctrl.alu, op_a, op_b, op_c, id.ctrl.is_rv32);
    trace_execute!(cpu.trace;
        rob_tag  = id.rob_tag.0,
        pc       = %crate::trace::Hex(id.pc),
        op_a     = %crate::trace::Hex(op_a),
        op_b     = %crate::trace::Hex(op_b),
        result   = %crate::trace::Hex(alu_out),
        fp_flags,
        "EX: ALU/FPU result"
    );

    let mut needs_flush = false;

    // Branch resolution
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

        cpu.branch_predictor.repair_history(id.ghr_snapshot);
        cpu.branch_predictor.restore_ras(id.ras_snapshot);
        // Defer branch predictor update to commit time to avoid
        // polluting tables with speculative/wrong-path data.
        rob.set_bp_update(
            id.rob_tag,
            id.pc,
            BpOutcome { taken, mispredicted },
            if taken { Some(actual_target) } else { None },
        );

        trace_branch!(cpu.trace;
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
            cpu.stats.speculative_branch_mispredictions += 1;
            cpu.pc = actual_next_pc;
            cpu.redirect_pending = true;
            needs_flush = true;
        } else {
            cpu.stats.speculative_branch_predictions += 1;
        }
    }

    // Jump resolution
    if id.ctrl.control_flow == ControlFlow::Jump {
        use crate::common::constants::OPCODE_MASK;
        let is_jalr = (id.inst & OPCODE_MASK) == opcodes::OP_JALR;
        let is_call = (id.inst & OPCODE_MASK) == opcodes::OP_JAL && id.rd == abi::REG_RA;
        let is_ret = is_jalr && id.rd == abi::REG_ZERO && id.rs1 == abi::REG_RA;

        let actual_target = if is_jalr {
            (fwd_a.wrapping_add(id.imm as u64)) & JALR_ALIGNMENT_MASK
        } else {
            id.pc.wrapping_add(id.imm as u64)
        };

        let predicted_target =
            if id.pred_taken { id.pred_target } else { id.pc.wrapping_add(id.inst_size.as_u64()) };

        let mispredicted = actual_target != predicted_target;

        // Defer branch predictor update to commit time
        rob.set_bp_update(
            id.rob_tag,
            id.pc,
            BpOutcome { taken: true, mispredicted },
            Some(actual_target),
        );

        trace_branch!(cpu.trace;
            event          = "resolve",
            pc             = %crate::trace::Hex(id.pc),
            rob_tag        = id.rob_tag.0,
            bp_type        = if is_ret { "JALR/RAS" } else if is_call { "JAL/call" } else { "JAL/JALR" },
            pred_taken     = id.pred_taken,
            pred_target    = %crate::trace::Hex(predicted_target),
            actual_taken   = true,
            actual_target  = %crate::trace::Hex(actual_target),
            mispredicted,
            "EX: jump resolved"
        );
        if mispredicted {
            cpu.stats.speculative_branch_mispredictions += 1;
            cpu.pc = actual_target;
            cpu.redirect_pending = true;
            needs_flush = true;
        } else {
            cpu.stats.speculative_branch_predictions += 1;
        }

        if is_call {
            cpu.branch_predictor.on_call(
                id.pc,
                id.pc.wrapping_add(id.inst_size.as_u64()),
                actual_target,
            );
        } else if is_ret {
            cpu.branch_predictor.on_return();
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
    };

    (result, needs_flush)
}

/// Handle system instructions (MRET, SRET, WFI, SFENCE.VMA, ECALL, CSR).
fn execute_system(
    cpu: &mut Cpu,
    id: RenameIssueEntry,
    rob: &mut Rob,
    fwd_a: u64,
    store_data: u64,
) -> (ExMem1Entry, bool) {
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
        };

    // MRET
    if id.ctrl.system_op == SystemOp::Mret {
        trace_trap!(cpu.trace;
            event       = "return",
            pc          = %crate::trace::Hex(id.pc),
            rob_tag     = id.rob_tag.0,
            insn        = "MRET",
            priv_mode   = ?cpu.privilege,
            mepc        = %crate::trace::Hex(cpu.csrs.mepc),
            mstatus     = %crate::trace::Hex(cpu.csrs.mstatus),
            "EX: MRET queued (privilege restore deferred to commit)"
        );
        return (make_result(0, id.ctrl), true);
    }

    // SRET
    if id.ctrl.system_op == SystemOp::Sret {
        let tsr = (cpu.csrs.mstatus >> 22) & 1;
        if cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tsr != 0 {
            trace_trap!(cpu.trace;
                event   = "illegal",
                pc      = %crate::trace::Hex(id.pc),
                rob_tag = id.rob_tag.0,
                insn    = "SRET",
                reason  = "TSR=1 in S-mode",
                "EX: SRET -> IllegalInstruction (TSR)"
            );
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
            return (make_result(0, id.ctrl), true);
        }
        trace_trap!(cpu.trace;
            event     = "return",
            pc        = %crate::trace::Hex(id.pc),
            rob_tag   = id.rob_tag.0,
            insn      = "SRET",
            priv_mode = ?cpu.privilege,
            sepc      = %crate::trace::Hex(cpu.csrs.sepc),
            mstatus   = %crate::trace::Hex(cpu.csrs.mstatus),
            "EX: SRET queued (privilege restore deferred to commit)"
        );
        return (make_result(0, id.ctrl), true);
    }

    // WFI: deferred to commit (like MRET/SRET), but check privilege here.
    if id.ctrl.system_op == SystemOp::Wfi {
        let tw = (cpu.csrs.mstatus >> 21) & 1;
        if cpu.privilege == crate::core::arch::mode::PrivilegeMode::User
            || (cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tw != 0)
        {
            trace_trap!(cpu.trace;
                event   = "illegal",
                pc      = %crate::trace::Hex(id.pc),
                insn    = "WFI",
                reason  = "U-mode or TW=1",
                "EX: WFI -> IllegalInstruction"
            );
            rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
        }
        return (make_result(0, id.ctrl), true);
    }

    // SFENCE.VMA
    //
    // Do absolutely nothing at execute time — no TLB flush, no redirect.
    // Preceding PTE-modifying stores may still be in the store buffer,
    // and flushing TLBs now would let in-flight instructions repopulate
    // them with stale translations.  The instruction flows through the
    // pipeline carrying its operands in SfenceVmaInfo.  At commit time,
    // the commit stage stalls until the store buffer is fully drained,
    // then flushes TLBs with proper ASID/vaddr granularity and triggers
    // a full pipeline squash.
    if id.ctrl.system_op == SystemOp::SfenceVma {
        let tvm = (cpu.csrs.mstatus >> 20) & 1;
        if cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tvm != 0 {
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
                },
                true,
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
            },
            false,
        );
    }

    // ECALL
    if id.inst == sys_ops::ECALL {
        use crate::core::arch::mode::PrivilegeMode;
        let trap = match cpu.privilege {
            PrivilegeMode::User => Trap::EnvironmentCallFromUMode,
            PrivilegeMode::Supervisor => Trap::EnvironmentCallFromSMode,
            PrivilegeMode::Machine => Trap::EnvironmentCallFromMMode,
        };
        trace_trap!(cpu.trace;
            event     = "take",
            pc        = %crate::trace::Hex(id.pc),
            rob_tag   = id.rob_tag.0,
            cause     = "ECALL",
            priv_mode = ?cpu.privilege,
            a7        = %crate::trace::Hex(cpu.regs.read(crate::isa::abi::REG_A7)),
            a0        = %crate::trace::Hex(cpu.regs.read(crate::isa::abi::REG_A0)),
            "EX: ECALL"
        );
        rob.fault(id.rob_tag, trap, ExceptionStage::Execute);
        return (make_result(0, id.ctrl), true);
    }

    // CSR operations
    if id.ctrl.csr_op != CsrOp::None {
        return execute_csr(cpu, id, rob, fwd_a, store_data);
    }

    // Unknown system instruction — pass through
    (make_result(0, id.ctrl), true)
}

/// Handle CSR operations.
#[allow(clippy::needless_pass_by_value)]
fn execute_csr(
    cpu: &mut Cpu,
    id: RenameIssueEntry,
    rob: &mut Rob,
    fwd_a: u64,
    store_data: u64,
) -> (ExMem1Entry, bool) {
    // SATP access check in S-mode with TVM=1
    if id.ctrl.csr_addr == crate::core::arch::csr::SATP
        && cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor
        && ((cpu.csrs.mstatus >> 20) & 1) != 0
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
            },
            true,
        );
    }

    // Counter-enable check for CYCLE/TIME/INSTRET (and their high-word aliases)
    {
        use crate::core::arch::csr as csr_addrs;
        use crate::core::arch::mode::PrivilegeMode;
        let counter_bit = if id.ctrl.csr_addr == csr_addrs::CYCLE {
            Some(0) // CY bit
        } else if id.ctrl.csr_addr == csr_addrs::TIME {
            Some(1) // TM bit
        } else if id.ctrl.csr_addr == csr_addrs::INSTRET {
            Some(2) // IR bit
        } else {
            None
        };
        if let Some(bit) = counter_bit {
            let mask = 1u64 << bit;
            let denied = match cpu.privilege {
                PrivilegeMode::Supervisor => (cpu.csrs.mcounteren & mask) == 0,
                PrivilegeMode::User => {
                    (cpu.csrs.mcounteren & mask) == 0 || (cpu.csrs.scounteren & mask) == 0
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
                    },
                    true,
                );
            }
        }
    }

    // Privilege check
    let csr_priv = id.ctrl.csr_addr.privilege_level() as u32;
    if (cpu.privilege.to_u8() as u32) < csr_priv {
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
            },
            true,
        );
    }

    // Read-only check
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
                },
                true,
            );
        }
    }

    // fp_flags are deferred to commit, but a CSR read of fflags/fcsr must
    // see the accumulated flags from all older (completed) instructions.
    // CSR instructions are serializing, so all older entries are complete.
    {
        use crate::core::arch::csr as csr_addrs;
        if id.ctrl.csr_addr == csr_addrs::FFLAGS
            || id.ctrl.csr_addr == csr_addrs::FCSR
            || id.ctrl.csr_addr == csr_addrs::FRM
        {
            let acc = rob.drain_fp_flags_before(id.rob_tag);
            cpu.csrs.fflags |= acc as u64;
        }
    }
    let old = cpu.csr_read(id.ctrl.csr_addr);
    let src = match id.ctrl.csr_op {
        CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => id.rs1.as_usize() as u64 & 0x1f,
        _ => fwd_a,
    };
    let new = match id.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => src,
        CsrOp::Rs | CsrOp::Rsi => old | src,
        CsrOp::Rc | CsrOp::Rci => old & !src,
        CsrOp::None => old,
    };

    trace_csr!(cpu.trace;
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
    rob.set_csr_update(
        id.rob_tag,
        CsrUpdate { addr: id.ctrl.csr_addr, old_val: old, new_val: new, applied: false },
    );

    cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
    cpu.redirect_pending = true;

    (
        ExMem1Entry {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            alu: old, // result = old CSR value for rd
            store_data,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: id.rd_phys,
            fp_flags: 0,
            sfence_vma: None,
        },
        true,
    )
}

/// Compute ALU/FPU result and return (result, `fp_flags`).
fn compute_alu(alu_op: AluOp, op_a: u64, op_b: u64, op_c: u64, is_rv32: bool) -> (u64, u8) {
    // FP conversions and moves that need special handling
    match alu_op {
        AluOp::FCvtSW
        | AluOp::FCvtSL
        | AluOp::FCvtSWU
        | AluOp::FCvtSLU
        | AluOp::FCvtSD
        | AluOp::FCvtDS
        | AluOp::FMvToF => {
            use crate::core::units::fpu::exception_flags::FpFlags;
            let mut flags: u8 = 0;
            let val = match alu_op {
                AluOp::FCvtSW => {
                    if is_rv32 {
                        // FCVT.S.W: i32 -> f32
                        let src = op_a as i32;
                        let result = src as f32;
                        if result as i32 != src {
                            flags = FpFlags::NX.bits();
                        }
                        Fpu::box_f32(result)
                    } else {
                        // FCVT.D.W: i32 -> f64 (always exact)
                        ((op_a as i32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSWU => {
                    if is_rv32 {
                        // FCVT.S.WU: u32 -> f32
                        let src = op_a as u32;
                        let result = src as f32;
                        if result as u32 != src {
                            flags = FpFlags::NX.bits();
                        }
                        Fpu::box_f32(result)
                    } else {
                        // FCVT.D.WU: u32 -> f64 (always exact)
                        ((op_a as u32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSL => {
                    let src = op_a as i64;
                    if is_rv32 {
                        // FCVT.S.L: i64 -> f32
                        let result = src as f32;
                        if result as i64 != src {
                            flags = FpFlags::NX.bits();
                        }
                        Fpu::box_f32(result)
                    } else {
                        // FCVT.D.L: i64 -> f64
                        let result = src as f64;
                        if result as i64 != src {
                            flags = FpFlags::NX.bits();
                        }
                        result.to_bits()
                    }
                }
                AluOp::FCvtSLU => {
                    let src = op_a;
                    if is_rv32 {
                        // FCVT.S.LU: u64 -> f32
                        let result = src as f32;
                        if result as u64 != src {
                            flags = FpFlags::NX.bits();
                        }
                        Fpu::box_f32(result)
                    } else {
                        // FCVT.D.LU: u64 -> f64
                        let result = src as f64;
                        if result as u64 != src {
                            flags = FpFlags::NX.bits();
                        }
                        result.to_bits()
                    }
                }
                AluOp::FCvtSD => {
                    // FCVT.S.D: f64 -> f32 (may be inexact)
                    use crate::core::units::fpu::nan_handling::box_f32_canon;
                    let val_d = f64::from_bits(op_a);
                    let val_s = val_d as f32;
                    // Inexact if round-trip doesn't match and neither is NaN
                    if !val_d.is_nan() && (val_s as f64).to_bits() != op_a {
                        flags = FpFlags::NX.bits();
                    }
                    box_f32_canon(val_s)
                }
                AluOp::FCvtDS => {
                    // FCVT.D.S: f32 -> f64 (always exact, but sNaN -> NV)
                    use crate::core::units::fpu::nan_handling::unbox_f32;
                    let val_s = unbox_f32(op_a);
                    // Check for signaling NaN: sNaN has quiet bit (bit 22) clear
                    if val_s.is_nan() && (val_s.to_bits() & (1 << 22)) == 0 {
                        flags = FpFlags::NV.bits();
                    }
                    let val_d = val_s as f64;
                    val_d.to_bits()
                }
                AluOp::FMvToF => {
                    // Bit move, no flags
                    if is_rv32 { Fpu::box_f32(f32::from_bits(op_a as u32)) } else { op_a }
                }
                _ => 0,
            };
            return (val, flags);
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
            | AluOp::FMvToX
    );

    if is_fp_op {
        let (result, fp_flags) = Fpu::execute_full(alu_op, op_a, op_b, op_c, is_rv32);
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
    use crate::soc::builder::System;

    #[test]
    fn test_execute_one_normal() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(!flush);
        assert_eq!(result.alu, 10); // rv1 (10) + 0
        assert_eq!(result.rob_tag, tag);
    }

    #[test]
    fn test_execute_trap_propagation() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (_result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(flush);
        let entry = rob.find_entry(tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
        assert!(entry.trap.is_some());
    }

    #[test]
    fn test_execute_fence_i() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (_result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(flush);
        assert!(cpu.redirect_pending);
        assert_eq!(cpu.pc, 0x1004);
    }

    #[test]
    fn test_execute_fp_trap_when_fs_zero() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut rob = Rob::new(4);

        cpu.csrs.mstatus &= !crate::core::arch::csr::MSTATUS_FS; // Clear FS bits

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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (_result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(flush);
        let entry = rob.find_entry(tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
    }

    #[test]
    fn test_execute_branch_misprediction() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (_result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(flush);
        assert!(cpu.redirect_pending);
        assert_eq!(cpu.pc, 0x1008); // Mispredicted, redirect to actual target
        let entry = rob.find_entry(tag).unwrap();
        assert!(entry.bp_outcome.mispredicted);
    }

    #[test]
    fn test_execute_jump_jalr() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
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
            ghr_snapshot: 0,
            ras_snapshot: 0,
        };

        let (_result, flush) = execute_one(&mut cpu, issue, &mut rob);
        assert!(flush);
        assert!(cpu.redirect_pending);

        let expected_target = (0x2000 + 0x15) & !1;
        assert_eq!(cpu.pc, expected_target);

        let entry = rob.find_entry(tag).unwrap();
        assert!(entry.bp_outcome.mispredicted);
    }
}
