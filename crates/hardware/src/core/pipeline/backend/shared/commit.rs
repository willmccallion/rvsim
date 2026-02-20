//! Commit Stage: retire instructions from ROB head.
//!
//! This stage retires the oldest instruction(s) from the ROB in program order:
//! 1. Write results to the register file.
//! 2. Apply deferred CSR writes.
//! 3. Mark store buffer entries as Committed.
//! 4. Handle traps/interrupts.
//! 5. Drain one committed store to memory per cycle.

use crate::common::Trap;
use crate::common::constants::{
    DELEG_MEIP_BIT, DELEG_MSIP_BIT, DELEG_MTIP_BIT, DELEG_SEIP_BIT, DELEG_SSIP_BIT, DELEG_STIP_BIT,
};
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::core::cpu::PC_TRACE_MAX;
use crate::core::pipeline::rob::{Rob, RobState};
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::signals::{AluOp, MemWidth};
use crate::core::pipeline::store_buffer::StoreBuffer;

/// Executes the Commit stage.
///
/// Retires up to `width` instructions from the ROB head per cycle.
/// Handles register writes, CSR application, trap dispatch, and
/// store buffer drain.
pub fn commit_stage(
    cpu: &mut Cpu,
    rob: &mut Rob,
    store_buffer: &mut StoreBuffer,
    scoreboard: &mut Scoreboard,
    width: usize,
) -> Option<(Trap, u64)> {
    let mut trap_event: Option<(Trap, u64)> = None;

    // Check for interrupts before committing.
    // Always check — even with an empty ROB (e.g., timer fired during a stall
    // with no instructions in-flight). Use cpu.pc as EPC when ROB is empty.
    {
        let epc = if cpu.wfi_waiting {
            cpu.wfi_pc
        } else if let Some(head) = rob.peek_head() {
            head.pc
        } else {
            cpu.pc // ROB empty: next instruction to fetch
        };

        let interrupt = check_interrupts(cpu);
        if let Some(interrupt_trap) = interrupt {
            cpu.wfi_waiting = false;
            if cpu.trace {
                eprintln!(
                    "CM  pc={:#x} * INTERRUPT DETECTED: {:?}",
                    epc, interrupt_trap
                );
            }
            trap_event = Some((interrupt_trap, epc));
        } else if cpu.wfi_waiting {
            // WFI wakeup without trap
            let pending = cpu.csrs.mip;
            let enabled = cpu.csrs.mie;
            if (pending & enabled) != 0 {
                cpu.wfi_waiting = false;
                cpu.pc = cpu.wfi_pc;
            }
        }
    }

    // If interrupt detected, don't commit — flush everything
    if trap_event.is_some() {
        return trap_event;
    }

    // Commit up to `width` entries from ROB head
    for _ in 0..width {
        let head = match rob.peek_head() {
            Some(h) => h,
            None => break,
        };

        if head.state == RobState::Issued {
            break; // Not ready yet
        }

        if head.state == RobState::Faulted {
            // Synchronous exception: take the trap
            let entry = rob.commit_head().unwrap();
            if cpu.trace {
                eprintln!(
                    "CM  pc={:#x} * SYNC TRAP: {:?}",
                    entry.pc,
                    entry.trap.as_ref().unwrap()
                );
            }
            trap_event = Some((entry.trap.unwrap(), entry.pc));
            break;
        }

        // Completed — retire
        let entry = rob.commit_head().unwrap();

        if cpu.trace {
            eprintln!("CM  pc={:#x} rob_tag={} COMMIT", entry.pc, entry.tag.0);
        }

        // Update PC trace
        cpu.pc_trace.push((entry.pc, entry.inst));
        if cpu.pc_trace.len() > PC_TRACE_MAX {
            cpu.pc_trace.remove(0);
        }

        // Statistics
        if entry.inst != 0 && entry.inst != 0x13 {
            cpu.stats.instructions_retired += 1;
            update_instruction_stats(cpu, &entry);
        }

        // Write to register file
        let val = entry.result;
        if entry.ctrl.fp_reg_write {
            cpu.regs.write_f(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, true, entry.tag);
            // Set FS to DIRTY when any FP register is written
            cpu.csrs.mstatus = (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            cpu.csrs.sstatus = (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            if cpu.trace {
                eprintln!("CM  pc={:#x} f{} <= {:#x}", entry.pc, entry.rd, val);
            }
        } else if entry.ctrl.reg_write && entry.rd != 0 {
            cpu.regs.write(entry.rd, val);
            scoreboard.clear_if_match(entry.rd, false, entry.tag);
            if cpu.trace {
                eprintln!("CM  pc={:#x} x{} <= {:#x}", entry.pc, entry.rd, val);
            }
        }

        // Apply deferred CSR write
        if let Some(csr_update) = entry.csr_update {
            cpu.csr_write(csr_update.addr, csr_update.new_val);
            if cpu.trace {
                eprintln!(
                    "CM  pc={:#x} CSR {:#x} <= {:#x}",
                    entry.pc, csr_update.addr, csr_update.new_val
                );
            }
            // CSR instructions are serializing — drain before committing more
            break;
        }

        // Handle MRET/SRET at commit (serializing instructions)
        if entry.ctrl.is_mret {
            cpu.do_mret();
            if cpu.trace {
                eprintln!("CM  pc={:#x} MRET -> PC={:#x}", entry.pc, cpu.pc);
            }
            break;
        }
        if entry.ctrl.is_sret {
            cpu.do_sret();
            if cpu.trace {
                eprintln!("CM  pc={:#x} SRET -> PC={:#x}", entry.pc, cpu.pc);
            }
            break;
        }

        // Mark store buffer entry as committed (for stores)
        if entry.ctrl.mem_write {
            store_buffer.mark_committed(entry.tag);
            // FP stores also set FS to DIRTY
            if entry.ctrl.rs2_fp {
                cpu.csrs.mstatus = (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                cpu.csrs.sstatus = (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
        }

        // Ensure x0 stays zero
        cpu.regs.write(0, 0);
    }

    // Drain one committed store to memory per cycle
    if let Some(store) = store_buffer.drain_one()
        && let Some(paddr) = store.paddr
    {
        let in_htif = cpu
            .htif_range
            .is_some_and(|(lo, hi)| paddr >= lo && paddr < hi);
        let is_ram = !in_htif && paddr >= cpu.ram_start && paddr < cpu.ram_end;
        if is_ram {
            let offset = (paddr - cpu.ram_start) as usize;
            unsafe {
                match store.width {
                    MemWidth::Byte => *cpu.ram_ptr.add(offset) = store.data as u8,
                    MemWidth::Half => {
                        (cpu.ram_ptr.add(offset) as *mut u16).write_unaligned(store.data as u16)
                    }
                    MemWidth::Word => {
                        (cpu.ram_ptr.add(offset) as *mut u32).write_unaligned(store.data as u32)
                    }
                    MemWidth::Double => {
                        (cpu.ram_ptr.add(offset) as *mut u64).write_unaligned(store.data)
                    }
                    _ => {}
                }
            }
        } else {
            match store.width {
                MemWidth::Byte => cpu.bus.bus.write_u8(paddr, store.data as u8),
                MemWidth::Half => cpu.bus.bus.write_u16(paddr, store.data as u16),
                MemWidth::Word => cpu.bus.bus.write_u32(paddr, store.data as u32),
                MemWidth::Double => cpu.bus.bus.write_u64(paddr, store.data),
                _ => {}
            }
        }

        if cpu.trace {
            eprintln!("CM  STORE DRAIN paddr={:#x} data={:#x}", paddr, store.data);
        }
    }

    trap_event
}

/// Checks for pending interrupts. Returns the trap if one should be taken.
fn check_interrupts(cpu: &Cpu) -> Option<Trap> {
    let mip = cpu.csrs.mip;
    let mie = cpu.csrs.mie;
    let mstatus = cpu.csrs.mstatus;

    let m_global_ie = (mstatus & csr::MSTATUS_MIE) != 0;
    let s_global_ie = (mstatus & csr::MSTATUS_SIE) != 0;

    let check = |bit: u64, enable_bit: u64, deleg_bit: u64| -> Option<Trap> {
        let pending = (mip & bit) != 0;
        let enabled = (mie & enable_bit) != 0;
        if !pending || !enabled {
            return None;
        }

        let delegated = (cpu.csrs.mideleg & deleg_bit) != 0;
        let target_priv = if delegated {
            PrivilegeMode::Supervisor
        } else {
            PrivilegeMode::Machine
        };

        if cpu.privilege.to_u8() < target_priv.to_u8() {
            return Some(TrapHandler::irq_to_trap(bit));
        }
        if cpu.privilege == target_priv {
            if target_priv == PrivilegeMode::Machine && m_global_ie {
                return Some(TrapHandler::irq_to_trap(bit));
            }
            if target_priv == PrivilegeMode::Supervisor && s_global_ie {
                return Some(TrapHandler::irq_to_trap(bit));
            }
        }
        None
    };

    check(csr::MIP_MEIP, csr::MIE_MEIP, 1 << DELEG_MEIP_BIT)
        .or_else(|| check(csr::MIP_MSIP, csr::MIE_MSIP, 1 << DELEG_MSIP_BIT))
        .or_else(|| check(csr::MIP_MTIP, csr::MIE_MTIE, 1 << DELEG_MTIP_BIT))
        .or_else(|| check(csr::MIP_SEIP, csr::MIE_SEIP, 1 << DELEG_SEIP_BIT))
        .or_else(|| check(csr::MIP_SSIP, csr::MIE_SSIP, 1 << DELEG_SSIP_BIT))
        .or_else(|| check(csr::MIP_STIP, csr::MIE_STIE, 1 << DELEG_STIP_BIT))
}

/// Updates instruction statistics based on the committed entry.
fn update_instruction_stats(cpu: &mut Cpu, entry: &crate::core::pipeline::rob::RobEntry) {
    if entry.ctrl.mem_read {
        if entry.ctrl.fp_reg_write {
            cpu.stats.inst_fp_load += 1;
        } else {
            cpu.stats.inst_load += 1;
        }
    } else if entry.ctrl.mem_write {
        if entry.ctrl.rs2_fp {
            cpu.stats.inst_fp_store += 1;
        } else {
            cpu.stats.inst_store += 1;
        }
    } else if entry.ctrl.branch || entry.ctrl.jump {
        cpu.stats.inst_branch += 1;
    } else if entry.ctrl.is_system {
        cpu.stats.inst_system += 1;
    } else {
        match entry.ctrl.alu {
            AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMul
            | AluOp::FMin
            | AluOp::FMax
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
            | AluOp::FMvToX
            | AluOp::FMvToF => cpu.stats.inst_fp_arith += 1,
            AluOp::FDiv | AluOp::FSqrt => cpu.stats.inst_fp_div_sqrt += 1,
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                cpu.stats.inst_fp_fma += 1
            }
            _ => cpu.stats.inst_alu += 1,
        }
    }
}
