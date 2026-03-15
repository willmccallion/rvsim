//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use crate::common::error::{ExceptionStage, Trap};
use crate::common::reg_idx::RegIdx;
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::rob::{BpOutcome, CsrUpdate, Rob};
use crate::core::pipeline::signals::{AluOp, ControlFlow, CsrOp, OpASrc, OpBSrc, SystemOp};
use crate::core::units::alu::Alu;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::fpu::Fpu;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use crate::isa::rv64i::{funct3, opcodes};
use crate::{trace_execute, trace_trap};

const FUNCT3_SHIFT: u32 = 12;
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;

/// Executes instructions in the in-order backend.
///
/// Takes issued instructions, performs ALU/FPU operations, resolves branches,
/// and produces `ExMem1Entry` results. CSR writes and `MRET`/`SRET` are recorded
/// in the ROB for deferred application at commit.
///
/// Returns `(results, needs_frontend_flush)`. When `needs_frontend_flush` is true,
/// the engine must flush the issue queue and frontend (branch misprediction,
/// CSR, MRET/SRET, FENCE.I, etc.).
pub fn execute_inorder(
    cpu: &mut Cpu,
    entries: Vec<RenameIssueEntry>,
    rob: &mut Rob,
    inflight_fp_flags: u8,
) -> (Vec<ExMem1Entry>, bool) {
    let mut results = Vec::with_capacity(entries.len());
    let mut flush_remaining = false;
    // Accumulates fp_flags from FP instructions executed in this batch,
    // so a later CSR read of fflags in the same cycle sees them.
    let mut batch_fp_flags: u8 = 0;

    for id in entries {
        if flush_remaining {
            break;
        }

        // Propagate traps from earlier stages
        if let Some(trap) = id.trap.clone() {
            trace_trap!(cpu.trace;
                event   = "propagate",
                stage   = "EX",
                pc      = %crate::trace::Hex(id.pc),
                rob_tag = id.rob_tag.0,
                trap    = ?trap,
                "EX: trap propagated from earlier stage"
            );
            rob.fault(id.rob_tag, trap, id.exception_stage.unwrap_or(ExceptionStage::Execute));
            results.push(ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: 0,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None, // trap is in ROB now
                exception_stage: None,
                rd_phys: PhysReg::default(),
                fp_flags: 0,
                sfence_vma: None,
            });
            flush_remaining = true;
            continue;
        }

        trace_execute!(cpu.trace;
            rob_tag  = id.rob_tag.0,
            pc       = %crate::trace::Hex(id.pc),
            inst     = %crate::trace::Hex32(id.inst),
            rd       = id.rd.as_usize(),
            rs1      = id.rs1.as_usize(),
            rv1      = %crate::trace::Hex(id.rv1),
            rs2      = id.rs2.as_usize(),
            rv2      = %crate::trace::Hex(id.rv2),
            imm      = id.imm,
            alu_op   = ?id.ctrl.alu,
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

        // FENCE.I: invalidate I-cache so subsequent fetches see prior stores.
        if id.ctrl.system_op == SystemOp::FenceI {
            let _ = cpu.l1_i_cache.invalidate_all();
            cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
            cpu.redirect_pending = true;
            flush_remaining = true;

            results.push(ExMem1Entry {
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
                rd_phys: PhysReg::default(),
                fp_flags: 0,
                sfence_vma: None,
            });
            continue;
        }

        // System instructions (FENCE is a NOP at execute — handled at commit only)
        if !matches!(id.ctrl.system_op, SystemOp::None | SystemOp::Fence) {
            // MRET: requires M-mode privilege (spec §3.3.2)
            if id.ctrl.system_op == SystemOp::Mret {
                if cpu.privilege != crate::core::arch::mode::PrivilegeMode::Machine {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }
                flush_remaining = true;
                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }

            // SRET: requires at least S-mode privilege (spec §3.3.2)
            // In S-mode, SRET is illegal if mstatus.TSR=1
            if id.ctrl.system_op == SystemOp::Sret {
                if cpu.privilege == crate::core::arch::mode::PrivilegeMode::User {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }
                let tsr = (cpu.csrs.mstatus >> 22) & 1;
                if cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tsr != 0 {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }

                flush_remaining = true;
                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }

            // WFI: deferred to commit (like MRET/SRET), but check privilege here.
            // WFI is illegal in U-mode, or in S-mode when mstatus.TW=1.
            if id.ctrl.system_op == SystemOp::Wfi {
                let tw = (cpu.csrs.mstatus >> 21) & 1;
                if cpu.privilege == crate::core::arch::mode::PrivilegeMode::User
                    || (cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor
                        && tw != 0)
                {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                }
                flush_remaining = true;
                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }

            // SFENCE.VMA
            if id.ctrl.system_op == SystemOp::SfenceVma {
                // In S-mode, SFENCE.VMA is illegal if mstatus.TVM=1
                let tvm = (cpu.csrs.mstatus >> 20) & 1;
                if cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor && tvm != 0 {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }

                cpu.clear_reservation();
                sfence_vma_flush(cpu, id.rs1, id.rs2, fwd_a, fwd_b);

                // SFENCE.VMA is a serializing fence: flush the frontend
                // so subsequent fetches use the updated TLB state.
                cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
                cpu.redirect_pending = true;
                flush_remaining = true;

                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: Some(crate::common::SfenceVmaInfo {
                        rs1_idx: id.rs1,
                        rs2_idx: id.rs2,
                        rs1_val: fwd_a,
                        rs2_val: fwd_b,
                    }),
                });
                continue;
            }

            // ECALL: always generate a trap and let commit handle it.
            // In direct mode, the trap handler reads the (now committed)
            // architectural registers for the syscall number and exit code.
            if id.inst == sys_ops::ECALL {
                use crate::core::arch::mode::PrivilegeMode;
                let trap = match cpu.privilege {
                    PrivilegeMode::User => Trap::EnvironmentCallFromUMode,
                    PrivilegeMode::Supervisor => Trap::EnvironmentCallFromSMode,
                    PrivilegeMode::Machine => Trap::EnvironmentCallFromMMode,
                };

                rob.fault(id.rob_tag, trap, ExceptionStage::Execute);
                flush_remaining = true;

                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }

            // CSR operations: compute old/new but defer write to commit
            if id.ctrl.csr_op != CsrOp::None {
                // In S-mode, SATP access is illegal if mstatus.TVM=1
                if id.ctrl.csr_addr == crate::core::arch::csr::SATP
                    && cpu.privilege == crate::core::arch::mode::PrivilegeMode::Supervisor
                    && ((cpu.csrs.mstatus >> 20) & 1) != 0
                {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }

                // Counter-enable check for CYCLE/TIME/INSTRET
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
                        let denied = match cpu.privilege {
                            PrivilegeMode::Supervisor => (cpu.csrs.mcounteren & mask) == 0,
                            PrivilegeMode::User => {
                                (cpu.csrs.mcounteren & mask) == 0
                                    || (cpu.csrs.scounteren & mask) == 0
                            }
                            PrivilegeMode::Machine => false,
                        };
                        if denied {
                            rob.fault(
                                id.rob_tag,
                                Trap::IllegalInstruction(id.inst),
                                ExceptionStage::Execute,
                            );
                            flush_remaining = true;
                            results.push(ExMem1Entry {
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
                                rd_phys: PhysReg::default(),
                                fp_flags: 0,
                                sfence_vma: None,
                            });
                            continue;
                        }
                    }
                }

                // Privilege check: CSR bits [9:8] encode minimum privilege level.
                let csr_priv = id.ctrl.csr_addr.privilege_level() as u32;
                if (cpu.privilege.to_u8() as u32) < csr_priv {
                    rob.fault(
                        id.rob_tag,
                        Trap::IllegalInstruction(id.inst),
                        ExceptionStage::Execute,
                    );
                    flush_remaining = true;
                    results.push(ExMem1Entry {
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
                        rd_phys: PhysReg::default(),
                        fp_flags: 0,
                        sfence_vma: None,
                    });
                    continue;
                }

                // Read-only check: CSR bits [11:10] == 0b11 means read-only.
                // CSRRW/CSRRWI always write. CSRRS/CSRRC/CSRRSI/CSRRCI write
                // only when rs1 (or uimm) != 0.
                let read_only = id.ctrl.csr_addr.is_read_only();
                if read_only {
                    let would_write = match id.ctrl.csr_op {
                        CsrOp::Rw | CsrOp::Rwi => true,
                        CsrOp::Rs | CsrOp::Rc => !id.rs1.is_zero(),
                        CsrOp::Rsi | CsrOp::Rci => (id.rs1.as_u8() & 0x1f) != 0,
                        CsrOp::None => false,
                    };
                    if would_write {
                        rob.fault(
                            id.rob_tag,
                            Trap::IllegalInstruction(id.inst),
                            ExceptionStage::Execute,
                        );
                        flush_remaining = true;
                        results.push(ExMem1Entry {
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
                            rd_phys: PhysReg::default(),
                            fp_flags: 0,
                            sfence_vma: None,
                        });
                        continue;
                    }
                }

                // Drain accumulated fp_flags from older ROB entries AND in-flight
                // pipeline latches into fflags before reading fflags/fcsr/frm,
                // so the CSR read sees flags from all older FP instructions.
                {
                    use crate::core::arch::csr as csr_addrs;
                    if id.ctrl.csr_addr == csr_addrs::FFLAGS
                        || id.ctrl.csr_addr == csr_addrs::FCSR
                        || id.ctrl.csr_addr == csr_addrs::FRM
                    {
                        let acc = rob.drain_fp_flags_before(id.rob_tag);
                        cpu.csrs.fflags |= (acc | inflight_fp_flags | batch_fp_flags) as u64;
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

                // Only generate a CSR write if the operation actually writes.
                // CSRRS/CSRRC with rs1=x0 and CSRRSI/CSRRCI with uimm=0 are
                // pure reads and must not trigger write side effects (spec §2.8).
                let would_write = match id.ctrl.csr_op {
                    CsrOp::Rw | CsrOp::Rwi => true,
                    CsrOp::Rs | CsrOp::Rc => !id.rs1.is_zero(),
                    CsrOp::Rsi | CsrOp::Rci => (id.rs1.as_u8() & 0x1f) != 0,
                    CsrOp::None => false,
                };
                if would_write {
                    rob.set_csr_update(
                        id.rob_tag,
                        CsrUpdate {
                            addr: id.ctrl.csr_addr,
                            old_val: old,
                            new_val: new,
                            applied: false,
                        },
                    );
                }

                // Flush frontend after CSR
                cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
                cpu.redirect_pending = true;
                flush_remaining = true;

                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }
        }

        // When mstatus.FS == OFF, all FP instructions trap as illegal.
        // This check is in execute (not decode) because a preceding CSR write
        // to mstatus may still be in-flight (deferred to commit) when the FP
        // instruction is decoded, causing a false positive.
        {
            let fs = (cpu.csrs.mstatus & crate::core::arch::csr::MSTATUS_FS) >> 13;
            let is_fp = id.ctrl.fp_reg_write || id.ctrl.rs1_fp || id.ctrl.rs2_fp || id.ctrl.rs3_fp;
            if fs == 0 && is_fp {
                rob.fault(id.rob_tag, Trap::IllegalInstruction(id.inst), ExceptionStage::Execute);
                flush_remaining = true;
                results.push(ExMem1Entry {
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
                    rd_phys: PhysReg::default(),
                    fp_flags: 0,
                    sfence_vma: None,
                });
                continue;
            }
        }

        // ALU / FPU execution
        let (alu_out, fp_flags) = compute_alu(id.ctrl.alu, op_a, op_b, op_c, id.ctrl.is_rv32);

        // FP exception flags are deferred to commit via the ROB entry
        // (applied by commit_stage in shared/commit.rs).
        batch_fp_flags |= fp_flags;

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
            // Defer branch predictor update to commit time
            rob.set_bp_update(
                id.rob_tag,
                id.pc,
                BpOutcome { taken, mispredicted },
                if taken { Some(actual_target) } else { None },
            );

            if mispredicted {
                cpu.stats.speculative_branch_mispredictions += 1;
                cpu.pc = actual_next_pc;
                cpu.redirect_pending = true;
                flush_remaining = true;
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

            let predicted_target = if id.pred_taken {
                id.pred_target
            } else {
                id.pc.wrapping_add(id.inst_size.as_u64())
            };

            let mispredicted = actual_target != predicted_target;

            // Defer branch predictor update to commit time
            rob.set_bp_update(
                id.rob_tag,
                id.pc,
                BpOutcome { taken: true, mispredicted },
                Some(actual_target),
            );

            if mispredicted {
                cpu.stats.speculative_branch_mispredictions += 1;
                cpu.pc = actual_target;
                cpu.redirect_pending = true;
                flush_remaining = true;
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

        results.push(ExMem1Entry {
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
            rd_phys: PhysReg::default(),
            fp_flags,
            sfence_vma: None,
        });
    }

    (results, flush_remaining)
}

/// Performs selective SFENCE.VMA TLB/cache flushing based on rs1 and rs2.
///
/// * `rs1_idx` == 0, `rs2_idx` == 0: flush all TLB entries
/// * `rs1_idx` != 0, `rs2_idx` == 0: flush entries matching the virtual address in `rs1`
/// * `rs1_idx` == 0, `rs2_idx` != 0: flush non-global entries matching the ASID in `rs2`
/// * `rs1_idx` != 0, `rs2_idx` != 0: flush the entry matching both vaddr and ASID
fn sfence_vma_flush(cpu: &mut Cpu, rs1_idx: RegIdx, rs2_idx: RegIdx, rs1_val: u64, rs2_val: u64) {
    use crate::common::constants::{PAGE_SHIFT, VPN_MASK};
    use crate::common::{Asid, Vpn};
    match (!rs1_idx.is_zero(), !rs2_idx.is_zero()) {
        (false, false) => {
            // Flush all
            cpu.mmu.dtlb.flush();
            cpu.mmu.itlb.flush();
            cpu.mmu.l2_tlb.flush();
            let _ = cpu.l1_d_cache.flush();
            let _ = cpu.l1_i_cache.invalidate_all();
        }
        (true, false) => {
            // Flush by virtual address only
            let vpn = Vpn::new((rs1_val >> PAGE_SHIFT) & VPN_MASK);
            cpu.mmu.dtlb.flush_vaddr(vpn);
            cpu.mmu.itlb.flush_vaddr(vpn);
            cpu.mmu.l2_tlb.flush_vaddr(vpn);
        }
        (false, true) => {
            // Flush by ASID only (non-global entries)
            let asid = Asid::new(rs2_val as u16);
            cpu.mmu.dtlb.flush_asid(asid);
            cpu.mmu.itlb.flush_asid(asid);
            cpu.mmu.l2_tlb.flush_asid(asid);
        }
        (true, true) => {
            // Flush by both virtual address and ASID
            let vpn = Vpn::new((rs1_val >> PAGE_SHIFT) & VPN_MASK);
            let asid = Asid::new(rs2_val as u16);
            cpu.mmu.dtlb.flush_vaddr_asid(vpn, asid);
            cpu.mmu.itlb.flush_vaddr_asid(vpn, asid);
            cpu.mmu.l2_tlb.flush_vaddr_asid(vpn, asid);
        }
    }
}

/// Computes the ALU/FPU result and returns `(result, fp_flags)`.
/// `fp_flags` is non-zero only for floating-point arithmetic operations.
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
            let val = match alu_op {
                AluOp::FCvtSW => {
                    if is_rv32 {
                        Fpu::box_f32((op_a as i32) as f32)
                    } else {
                        ((op_a as i32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSWU => {
                    if is_rv32 {
                        Fpu::box_f32((op_a as u32) as f32)
                    } else {
                        ((op_a as u32) as f64).to_bits()
                    }
                }
                AluOp::FCvtSL => {
                    if is_rv32 {
                        Fpu::box_f32((op_a as i64) as f32)
                    } else {
                        ((op_a as i64) as f64).to_bits()
                    }
                }
                AluOp::FCvtSLU => {
                    if is_rv32 {
                        Fpu::box_f32(op_a as f32)
                    } else {
                        (op_a as f64).to_bits()
                    }
                }
                AluOp::FCvtSD => {
                    use crate::core::units::fpu::nan_handling::box_f32_canon;
                    let val_d = f64::from_bits(op_a);
                    let val_s = val_d as f32;
                    box_f32_canon(val_s)
                }
                AluOp::FCvtDS => {
                    let val_s = f32::from_bits(op_a as u32);
                    let val_d = val_s as f64;
                    val_d.to_bits()
                }
                AluOp::FMvToF => {
                    if is_rv32 {
                        Fpu::box_f32(f32::from_bits(op_a as u32))
                    } else {
                        op_a
                    }
                }
                _ => 0,
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
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_sfence_vma_flush() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        // Just ensure no panics for the different branches
        sfence_vma_flush(&mut cpu, RegIdx::new(0), RegIdx::new(0), 0, 0);
        sfence_vma_flush(&mut cpu, RegIdx::new(1), RegIdx::new(0), 0x1000, 0);
        sfence_vma_flush(&mut cpu, RegIdx::new(0), RegIdx::new(1), 0, 1);
        sfence_vma_flush(&mut cpu, RegIdx::new(1), RegIdx::new(1), 0x1000, 1);
    }

    #[test]
    fn test_compute_alu_fp_conversions() {
        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, false);
        assert_eq!(res, (1.0f64).to_bits());
        assert_eq!(flags, 0);

        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, true);
        assert_eq!(res, 0xFFFF_FFFF_0000_0000 | (1.0f32).to_bits() as u64); // nan-boxed
        assert_eq!(flags, 0);

        let (res, flags) = compute_alu(AluOp::FMvToF, 42, 0, 0, false);
        assert_eq!(res, 42);
        assert_eq!(flags, 0);
    }
}
