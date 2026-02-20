//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{CsrUpdate, Rob};
use crate::core::pipeline::signals::{AluOp, CsrOp, OpASrc, OpBSrc};
use crate::core::units::alu::Alu;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::fpu::Fpu;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use crate::isa::rv64i::{funct3, opcodes};

const FUNCT3_SHIFT: u32 = 12;
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;

/// Executes instructions in the in-order backend.
///
/// Takes issued instructions, performs ALU/FPU operations, resolves branches,
/// and produces ExMem1Entry results. CSR writes and MRET/SRET are recorded
/// in the ROB for deferred application at commit.
///
/// Returns `(results, needs_frontend_flush)`. When `needs_frontend_flush` is true,
/// the engine must flush the issue queue and frontend (branch misprediction,
/// CSR, MRET/SRET, FENCE.I, etc.).
pub fn execute_inorder(
    cpu: &mut Cpu,
    entries: Vec<RenameIssueEntry>,
    rob: &mut Rob,
) -> (Vec<ExMem1Entry>, bool) {
    let mut results = Vec::with_capacity(entries.len());
    let mut flush_remaining = false;

    for id in entries {
        if flush_remaining {
            break;
        }

        // Propagate traps from earlier stages
        if let Some(trap) = id.trap.clone() {
            if cpu.trace {
                eprintln!("EX  pc={:#x} # TRAP: {:?}", id.pc, trap);
            }
            rob.fault(
                id.rob_tag,
                trap,
                id.exception_stage.unwrap_or(ExceptionStage::Execute),
            );
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
            });
            flush_remaining = true;
            continue;
        }

        if cpu.trace {
            eprintln!("EX  pc={:#x} rob_tag={}", id.pc, id.rob_tag.0);
        }

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

        // FENCE.I: flush caches and frontend
        if id.ctrl.is_fence_i {
            cpu.l1_d_cache.flush();
            cpu.l1_i_cache.flush();
            cpu.pc = id.pc.wrapping_add(id.inst_size);
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
            });
            continue;
        }

        // System instructions
        if id.ctrl.is_system {
            // MRET: deferred to commit, but flush frontend
            if id.ctrl.is_mret {
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
                });
                continue;
            }

            // SRET: deferred to commit, but flush frontend
            // In S-mode, SRET is illegal if mstatus.TSR=1
            if id.ctrl.is_sret {
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
                });
                continue;
            }

            // WFI — Wait For Interrupt
            //
            // Per RISC-V spec, WFI stalls the hart until an interrupt might
            // need servicing.  The commit stage checks mip & mie each cycle
            // and wakes us when one arrives.
            //
            // If mie == 0 (no interrupts enabled) AND nothing is already
            // pending in mip, then no source can wake the hart. Treat WFI
            // as a NOP to avoid deadlock (e.g. OpenSBI early boot before
            // timer setup).  Once software has enabled at least one interrupt
            // source we enter the real waiting state.
            //
            // WFI is illegal in U-mode, or in S-mode when mstatus.TW=1.
            if id.inst == sys_ops::WFI {
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
                    flush_remaining = true;
                } else if cpu.csrs.mie != 0 || cpu.csrs.mip != 0 {
                    // At least one interrupt source is enabled or pending —
                    // enter the waiting state so the commit stage can wake us.
                    cpu.wfi_waiting = true;
                    cpu.wfi_pc = id.pc.wrapping_add(id.inst_size);
                    flush_remaining = true;
                } else {
                    // Nothing enabled, nothing pending — NOP (advance past WFI)
                    cpu.pc = id.pc.wrapping_add(id.inst_size);
                    cpu.redirect_pending = true;
                    flush_remaining = true;
                }

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
                });
                continue;
            }

            // SFENCE.VMA
            if (id.inst & 0xFE007FFF) == sys_ops::SFENCE_VMA {
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
                    });
                    continue;
                }

                cpu.clear_reservation();
                cpu.mmu.dtlb.flush();
                cpu.mmu.itlb.flush();
                cpu.l1_d_cache.flush();
                cpu.l1_i_cache.flush();

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
                    });
                    continue;
                }

                // Privilege check: CSR bits [9:8] encode minimum privilege level.
                let csr_priv = (id.ctrl.csr_addr >> 8) & 3;
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
                    });
                    continue;
                }

                // Read-only check: CSR bits [11:10] == 0b11 means read-only.
                // CSRRW/CSRRWI always write. CSRRS/CSRRC/CSRRSI/CSRRCI write
                // only when rs1 (or uimm) != 0.
                let read_only = (id.ctrl.csr_addr >> 10) & 3 == 3;
                if read_only {
                    let would_write = match id.ctrl.csr_op {
                        CsrOp::Rw | CsrOp::Rwi => true,
                        CsrOp::Rs | CsrOp::Rc => id.rs1 != 0,
                        CsrOp::Rsi | CsrOp::Rci => (id.rs1 & 0x1f) != 0,
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
                        });
                        continue;
                    }
                }

                let old = cpu.csr_read(id.ctrl.csr_addr);
                let src = match id.ctrl.csr_op {
                    CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => (id.rs1 as u64) & 0x1f,
                    _ => fwd_a,
                };
                let new = match id.ctrl.csr_op {
                    CsrOp::Rw | CsrOp::Rwi => src,
                    CsrOp::Rs | CsrOp::Rsi => old | src,
                    CsrOp::Rc | CsrOp::Rci => old & !src,
                    CsrOp::None => old,
                };

                // Store the deferred CSR update in the ROB
                rob.set_csr_update(
                    id.rob_tag,
                    CsrUpdate {
                        addr: id.ctrl.csr_addr,
                        old_val: old,
                        new_val: new,
                    },
                );

                // Flush frontend after CSR
                cpu.pc = id.pc.wrapping_add(id.inst_size);
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
                });
                continue;
            }
        }

        // ALU / FPU execution
        let (alu_out, fp_flags) = compute_alu(id.ctrl.alu, op_a, op_b, op_c, id.ctrl.is_rv32);

        // Accumulate FP exception flags into fcsr.fflags
        if fp_flags != 0 {
            cpu.csrs.fflags |= fp_flags as u64;
            // Writing fflags makes FP state dirty
            use crate::core::arch::csr;
            cpu.csrs.mstatus = (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            cpu.csrs.sstatus = (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
        }

        // Branch resolution
        if id.ctrl.branch {
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
            let fallthrough = id.pc.wrapping_add(id.inst_size);

            let predicted_target = if id.pred_taken {
                id.pred_target
            } else {
                fallthrough
            };
            let actual_next_pc = if taken { actual_target } else { fallthrough };

            let mispredicted = predicted_target != actual_next_pc;

            cpu.branch_predictor.repair_history(id.ghr_snapshot);
            cpu.branch_predictor.update_branch(
                id.pc,
                taken,
                if taken { Some(actual_target) } else { None },
            );

            if mispredicted {
                cpu.stats.branch_mispredictions += 1;
                cpu.stats.stalls_control += 2;
                cpu.pc = actual_next_pc;
                cpu.redirect_pending = true;
                flush_remaining = true;
            } else {
                cpu.stats.branch_predictions += 1;
            }
        }

        // Jump resolution
        if id.ctrl.jump {
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
                id.pc.wrapping_add(id.inst_size)
            };

            if actual_target != predicted_target {
                cpu.stats.branch_mispredictions += 1;
                cpu.stats.stalls_control += 2;
                cpu.pc = actual_target;
                cpu.redirect_pending = true;
                flush_remaining = true;
            } else {
                cpu.stats.branch_predictions += 1;
            }

            if is_call {
                cpu.branch_predictor.on_call(
                    id.pc,
                    id.pc.wrapping_add(id.inst_size),
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
        });
    }

    (results, flush_remaining)
}

/// Computes the ALU/FPU result and returns (result, fp_flags).
/// fp_flags is non-zero only for floating-point arithmetic operations.
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
