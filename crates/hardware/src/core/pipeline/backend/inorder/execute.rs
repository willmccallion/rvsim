//! In-Order Execute Unit: single ALU/FPU/BRU execution.
//!
//! This stage performs arithmetic, branch resolution, and system instruction
//! handling. CSR writes and MRET/SRET are deferred to commit via the ROB.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::rob::{BpOutcome, CsrUpdate, Rob};
use crate::core::pipeline::signals::{
    AluOp, ControlFlow, CsrOp, OpASrc, OpBSrc, SystemOp, VectorOp,
};
use crate::core::units::alu::Alu;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::fpu::Fpu;
use crate::core::units::fpu::rounding_modes::RoundingMode;
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
                vec_mem: None,
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

        // FENCE.I: flush the pipeline so younger instructions are squashed.
        // The I-cache flush is deferred to COMMIT time (after store drain)
        // to ensure that all prior stores are visible in RAM before the
        // I-cache refills with the new data.
        if id.ctrl.system_op == SystemOp::FenceI {
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
                vec_mem: None,
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
                        vec_mem: None,
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
                    vec_mem: None,
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
                        vec_mem: None,
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
                        vec_mem: None,
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
                    vec_mem: None,
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
                    vec_mem: None,
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
                        vec_mem: None,
                    });
                    continue;
                }

                // SFENCE.VMA: Do NOT flush TLBs or clear reservation here.
                // Preceding PTE-modifying stores may still be in the store
                // buffer; flushing TLBs now would let in-flight fetches
                // repopulate them with stale translations. The commit stage
                // stalls until the store buffer drains, then flushes TLBs
                // with proper ASID/vaddr granularity.
                //
                // Flush the frontend so subsequent fetches are deferred
                // until after the commit-time TLB flush.
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
                    vec_mem: None,
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
                    vec_mem: None,
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
                        vec_mem: None,
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
                                vec_mem: None,
                            });
                            continue;
                        }
                    }
                }

                // Existence check: non-existent CSRs must trap (spec §2.2).
                if !cpu.is_valid_csr(id.ctrl.csr_addr) {
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
                        vec_mem: None,
                    });
                    continue;
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
                        vec_mem: None,
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
                            vec_mem: None,
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
                    vec_mem: None,
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
                    vec_mem: None,
                });
                continue;
            }
        }

        // Vector execution: dispatch to VPU, skip normal ALU path.
        // Vector ops are serializing (system_op = System) so they drain the pipeline
        // before executing here. The VPR is read/written directly at execute time;
        // commit will set mstatus.VS=dirty and vstart=0.
        if id.ctrl.vec_op != VectorOp::None {
            let alu_out = crate::core::units::vpu::execute::execute_vec_op(cpu, &id);
            // vsetvl family writes scalar rd — alu_out is the new vl.
            // Other vector ops produce no scalar result (alu_out = 0).
            // Flush pipeline after serializing instruction.
            cpu.pc = id.pc.wrapping_add(id.inst_size.as_u64());
            cpu.redirect_pending = true;
            flush_remaining = true;

            rob.complete(id.rob_tag, alu_out);
            results.push(ExMem1Entry {
                rob_tag: id.rob_tag,
                pc: id.pc,
                inst: id.inst,
                inst_size: id.inst_size,
                rd: id.rd,
                alu: alu_out,
                store_data: 0,
                ctrl: id.ctrl,
                trap: None,
                exception_stage: None,
                rd_phys: PhysReg::default(),
                fp_flags: 0,
                sfence_vma: None,
                vec_mem: None,
            });
            continue;
        }

        // ALU / FPU execution
        let fp_rm = id
            .ctrl
            .fp_rm
            .or_else(|| RoundingMode::from_bits(cpu.csrs.frm as u8));
        let (alu_out, fp_flags) = compute_alu(
            id.ctrl.alu,
            op_a,
            op_b,
            op_c,
            id.ctrl.is_f16,
            id.ctrl.is_rv32,
            fp_rm,
        );

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

            // Defer branch predictor update to commit time
            rob.set_bp_update(
                id.rob_tag,
                id.pc,
                BpOutcome { taken, mispredicted },
                if taken { Some(actual_target) } else { None },
                id.ghr_snapshot,
            );

            if mispredicted {
                // Restore GHR to the snapshot (pre-speculation state), then
                // push the actual outcome so subsequent fetches see correct history.
                cpu.branch_predictor.repair_history(&id.ghr_snapshot);
                cpu.branch_predictor.speculate(id.pc, taken);
                cpu.branch_predictor.restore_ras(id.ras_snapshot);
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
            let rd_link = id.rd == abi::REG_RA || id.rd == abi::REG_T0;
            let rs1_link = is_jalr && (id.rs1 == abi::REG_RA || id.rs1 == abi::REG_T0);

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

            // Store the jump target in the ROB for committed_next_pc tracking,
            // but don't set bp_update — jumps are unconditional and should not
            // train the direction predictor.
            rob.set_bp_target(id.rob_tag, actual_target);

            // Update BTB directly — jumps are unconditional, don't train direction predictor.
            // Skip for calls — on_call already updates the BTB.
            if !rd_link {
                cpu.branch_predictor.update_btb(id.pc, actual_target);
            }

            if mispredicted {
                cpu.branch_predictor.repair_history(&id.ghr_snapshot);
                cpu.branch_predictor.restore_ras(id.ras_snapshot);
                cpu.stats.speculative_branch_mispredictions += 1;
                cpu.pc = actual_target;
                cpu.redirect_pending = true;
                flush_remaining = true;
            } else {
                cpu.stats.speculative_branch_predictions += 1;
            }

            // RAS management per RISC-V spec Table 2.1:
            // Both x1 (ra) and x5 (t0) are link registers.
            let ret_addr = id.pc.wrapping_add(id.inst_size.as_u64());
            if rd_link && rs1_link && id.rd != id.rs1 {
                // Coroutine swap: pop then push
                cpu.branch_predictor.on_return();
                cpu.branch_predictor.on_call(id.pc, ret_addr, actual_target);
            } else if rd_link {
                // Call (JAL/JALR with rd in {x1, x5})
                cpu.branch_predictor.on_call(id.pc, ret_addr, actual_target);
            } else if rs1_link {
                // Return (JALR with rs1 in {x1, x5}, rd not a link register)
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
            vec_mem: None,
        });
    }

    (results, flush_remaining)
}

/// Computes the ALU/FPU result and returns `(result, fp_flags)`.
/// `fp_flags` is non-zero only for floating-point arithmetic operations.
fn compute_alu(
    alu_op: AluOp,
    op_a: u64,
    op_b: u64,
    op_c: u64,
    is_f16: bool,
    is_rv32: bool,
    fp_rm: Option<RoundingMode>,
) -> (u64, u8) {
    // FP conversions and moves that need special handling.
    // Int-to-float and float-to-float conversions can raise FP exception
    // flags (INEXACT, OVERFLOW, etc.), so we use the host FPU to detect them.
    match alu_op {
        AluOp::FCvtSW
        | AluOp::FCvtSL
        | AluOp::FCvtSWU
        | AluOp::FCvtSLU
        | AluOp::FCvtSD
        | AluOp::FCvtDS
        | AluOp::FCvtSH
        | AluOp::FCvtDH
            // When is_f16 is set, int↔f16 and f16↔f16 ops go through
            // execute_full_rm → execute_f16 for software rounding. Only
            // non-f16-target conversions are handled inline here.
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
                    // Half → single: lossless, just rebox.
                    let val_s = f16_to_f32(unbox_f16(op_a));
                    box_f32_canon(val_s)
                }
                AluOp::FCvtDH => {
                    // Half → double: lossless.
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
            // Bit-level move — no FP exceptions possible.
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
mod tests {
    use super::*;

    #[test]
    fn test_compute_alu_fp_conversions() {
        let rne = Some(RoundingMode::Rne);

        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, false, false, rne);
        assert_eq!(res, (1.0f64).to_bits());
        assert_eq!(flags, 0);

        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, false, true, rne);
        assert_eq!(res, 0xFFFF_FFFF_0000_0000 | (1.0f32).to_bits() as u64); // nan-boxed
        assert_eq!(flags, 0);

        let (res, flags) = compute_alu(AluOp::FMvToF, 42, 0, 0, false, false, rne);
        assert_eq!(res, 42);
        assert_eq!(flags, 0);
    }
}
