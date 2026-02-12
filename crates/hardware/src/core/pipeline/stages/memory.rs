//! Memory Access (MEM) Stage.
//!
//! This module implements the fourth stage of the instruction pipeline.
//! It handles Load/Store operations, performs virtual-to-physical address
//! translation via the MMU, and executes Atomic Memory Operations (AMOs).
//! It also manages data alignment and access faults.

use crate::common::{AccessType, TranslationResult, Trap, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::MemWbEntry;
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::units::lsu::Lsu;

/// Executes the memory stage of the pipeline.
///
/// Handles load and store operations, performs address translation,
/// executes atomic memory operations (AMO), and manages memory access
/// alignment checks. Updates the MEM/WB pipeline latch with memory results.
///
/// # Arguments
///
/// * `cpu` - Mutable reference to the CPU state
///
/// # Behavior
///
/// - Translates virtual addresses to physical addresses via MMU
/// - Performs load operations with sign extension and width conversion
/// - Executes store operations with proper width handling
/// - Implements atomic operations (LR, SC, AMO variants)
/// - Handles memory access faults and alignment exceptions
/// - Manages load reservation tracking for atomic operations
pub fn mem_stage(cpu: &mut Cpu) {
    let mut ex_entries = std::mem::take(&mut cpu.ex_mem.entries);

    let mut mem_results = std::mem::take(&mut cpu.mem_wb_shadow);
    mem_results.clear();

    let mut flush_remaining = false;

    for ex in ex_entries.drain(..) {
        if flush_remaining {
            break;
        }

        let mut ld = 0;
        let mut trap = ex.trap.clone();

        if trap.is_some() && cpu.trace {
            eprintln!("MEM pc={:#x} # TRAP: {:?}", ex.pc, trap.as_ref().unwrap());
        }

        if ex.ctrl.mem_read || ex.ctrl.mem_write {
            let align_mask = match ex.ctrl.width {
                MemWidth::Byte => 0,
                MemWidth::Half => 1,
                MemWidth::Word => 3,
                MemWidth::Double => 7,
                _ => 0,
            };

            if (ex.alu & align_mask) != 0 {
                let potential_trap = if ex.ctrl.mem_read {
                    Trap::LoadAddressMisaligned(ex.alu)
                } else {
                    Trap::StoreAddressMisaligned(ex.alu)
                };

                if cpu.trace {
                    eprintln!(
                        "MEM pc={:#x} # WARNING: Ignored {:?}",
                        ex.pc, potential_trap
                    );
                }
            }
        }

        if trap.is_none() && (ex.ctrl.mem_read || ex.ctrl.mem_write) {
            let access_type = if ex.ctrl.mem_write {
                AccessType::Write
            } else {
                AccessType::Read
            };

            let TranslationResult {
                paddr,
                cycles,
                trap: fault,
            } = cpu.translate(VirtAddr::new(ex.alu), access_type);
            cpu.stall_cycles += cycles;

            if let Some(t) = fault {
                if cpu.trace {
                    eprintln!("MEM pc={:#x} # TRAP: {:?} (addr={:#x})", ex.pc, t, ex.alu);
                }
                trap = Some(t);
            } else {
                if cpu.trace {
                    if ex.ctrl.mem_read {
                        eprintln!("MEM pc={:#x} LOAD addr={:#x}", ex.pc, ex.alu);
                    } else if ex.ctrl.mem_write {
                        eprintln!(
                            "MEM pc={:#x} STORE addr={:#x} data={:#x}",
                            ex.pc, ex.alu, ex.store_data
                        );
                    }
                }
                if paddr.val() >= cpu.mmio_base {
                    let lat = cpu.simulate_memory_access(paddr, access_type);
                    cpu.stall_cycles += lat;
                } else if ex.ctrl.mem_write {
                    let addr = paddr.val();
                    if addr >= 0x10001000 && addr < 0x10002000 {
                        cpu.l1_d_cache.flush();
                        cpu.l2_cache.flush();
                        cpu.l3_cache.flush();
                    }
                }

                let raw_paddr = paddr.val();
                let is_ram = raw_paddr >= cpu.ram_start && raw_paddr < cpu.ram_end;
                let ram_offset = if is_ram {
                    (raw_paddr - cpu.ram_start) as usize
                } else {
                    0
                };

                if ex.ctrl.atomic_op != AtomicOp::None {
                    match ex.ctrl.atomic_op {
                        AtomicOp::Lr => {
                            ld = match ex.ctrl.width {
                                MemWidth::Word => {
                                    (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                                }
                                MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                                _ => 0,
                            };
                            cpu.load_reservation = Some(raw_paddr);
                        }
                        AtomicOp::Sc => {
                            if cpu.load_reservation == Some(raw_paddr) {
                                match ex.ctrl.width {
                                    MemWidth::Word => {
                                        cpu.bus.bus.write_u32(raw_paddr, ex.store_data as u32)
                                    }
                                    MemWidth::Double => {
                                        cpu.bus.bus.write_u64(raw_paddr, ex.store_data)
                                    }
                                    _ => {}
                                }
                                ld = 0;
                            } else {
                                ld = 1;
                            }
                            cpu.load_reservation = None;
                        }
                        _ => {
                            let old_val = match ex.ctrl.width {
                                MemWidth::Word => {
                                    (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                                }
                                MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                                _ => 0,
                            };

                            let new_val = Lsu::atomic_alu(
                                ex.ctrl.atomic_op,
                                old_val,
                                ex.store_data,
                                ex.ctrl.width,
                            );

                            match ex.ctrl.width {
                                MemWidth::Word => cpu.bus.bus.write_u32(raw_paddr, new_val as u32),
                                MemWidth::Double => cpu.bus.bus.write_u64(raw_paddr, new_val),
                                _ => {}
                            }

                            ld = old_val;
                            if cpu.load_reservation == Some(raw_paddr) {
                                cpu.load_reservation = None;
                            }
                        }
                    }
                } else {
                    if ex.ctrl.mem_read {
                        ld = if is_ram {
                            // SAFETY: This read operation is safe because:
                            // 1. `is_ram` is true, meaning the address was validated to be within RAM bounds
                            // 2. `ram_offset` is computed from validated physical address and ram_start
                            // 3. `ram_ptr` points to valid, initialized memory allocated during CPU construction
                            // 4. `read_unaligned()` safely handles potential misalignment for multi-byte reads
                            // 5. Each read size (1/2/4/8 bytes) is within bounds as verified by address translation
                            // 6. Sign extension operations preserve correctness for signed loads
                            // 7. Memory access permissions have been validated by MMU/PMP checks
                            unsafe {
                                match (ex.ctrl.width, ex.ctrl.signed_load) {
                                    (MemWidth::Byte, true) => {
                                        (*cpu.ram_ptr.add(ram_offset) as i8) as i64 as u64
                                    }
                                    (MemWidth::Half, true) => {
                                        ((cpu.ram_ptr.add(ram_offset) as *const u16)
                                            .read_unaligned()
                                            as i16) as i64
                                            as u64
                                    }
                                    (MemWidth::Word, true) => {
                                        ((cpu.ram_ptr.add(ram_offset) as *const u32)
                                            .read_unaligned()
                                            as i32) as i64
                                            as u64
                                    }
                                    (MemWidth::Byte, false) => *cpu.ram_ptr.add(ram_offset) as u64,
                                    (MemWidth::Half, false) => {
                                        (cpu.ram_ptr.add(ram_offset) as *const u16).read_unaligned()
                                            as u64
                                    }
                                    (MemWidth::Word, false) => {
                                        (cpu.ram_ptr.add(ram_offset) as *const u32).read_unaligned()
                                            as u64
                                    }
                                    (MemWidth::Double, _) => {
                                        (cpu.ram_ptr.add(ram_offset) as *const u64).read_unaligned()
                                    }
                                    _ => 0,
                                }
                            }
                        } else {
                            match (ex.ctrl.width, ex.ctrl.signed_load) {
                                (MemWidth::Byte, true) => {
                                    (cpu.bus.bus.read_u8(raw_paddr) as i8) as i64 as u64
                                }
                                (MemWidth::Half, true) => {
                                    (cpu.bus.bus.read_u16(raw_paddr) as i16) as i64 as u64
                                }
                                (MemWidth::Word, true) => {
                                    (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                                }
                                (MemWidth::Byte, false) => cpu.bus.bus.read_u8(raw_paddr) as u64,
                                (MemWidth::Half, false) => cpu.bus.bus.read_u16(raw_paddr) as u64,
                                (MemWidth::Word, false) => cpu.bus.bus.read_u32(raw_paddr) as u64,
                                (MemWidth::Double, _) => cpu.bus.bus.read_u64(raw_paddr),
                                _ => 0,
                            }
                        };

                        if ex.ctrl.fp_reg_write && matches!(ex.ctrl.width, MemWidth::Word) {
                            ld |= 0xFFFF_FFFF_0000_0000;
                        }
                    } else if ex.ctrl.mem_write {
                        if cpu.load_reservation == Some(raw_paddr) {
                            cpu.load_reservation = None;
                        }

                        if is_ram {
                            // SAFETY: This write operation is safe because:
                            // 1. `is_ram` is true, meaning the address was validated to be within RAM bounds
                            // 2. `ram_offset` is computed from validated physical address and ram_start
                            // 3. `ram_ptr` points to valid, mutable memory allocated during CPU construction
                            // 4. `write_unaligned()` safely handles potential misalignment for multi-byte writes
                            // 5. Each write size (1/2/4/8 bytes) is within bounds as verified by address translation
                            // 6. Memory access permissions (write access) have been validated by MMU/PMP checks
                            // 7. Load reservation has been cleared to maintain memory ordering semantics
                            unsafe {
                                match ex.ctrl.width {
                                    MemWidth::Byte => {
                                        *cpu.ram_ptr.add(ram_offset) = ex.store_data as u8
                                    }
                                    MemWidth::Half => (cpu.ram_ptr.add(ram_offset) as *mut u16)
                                        .write_unaligned(ex.store_data as u16),
                                    MemWidth::Word => (cpu.ram_ptr.add(ram_offset) as *mut u32)
                                        .write_unaligned(ex.store_data as u32),
                                    MemWidth::Double => (cpu.ram_ptr.add(ram_offset) as *mut u64)
                                        .write_unaligned(ex.store_data),
                                    _ => {}
                                }
                            }
                        } else {
                            match ex.ctrl.width {
                                MemWidth::Byte => {
                                    cpu.bus.bus.write_u8(raw_paddr, ex.store_data as u8)
                                }
                                MemWidth::Half => {
                                    cpu.bus.bus.write_u16(raw_paddr, ex.store_data as u16)
                                }
                                MemWidth::Word => {
                                    cpu.bus.bus.write_u32(raw_paddr, ex.store_data as u32)
                                }
                                MemWidth::Double => {
                                    cpu.bus.bus.write_u64(raw_paddr, ex.store_data);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        } else if cpu.trace {
            eprintln!("MEM pc={:#x}", ex.pc);
        }

        mem_results.push(MemWbEntry {
            pc: ex.pc,
            inst: ex.inst,
            inst_size: ex.inst_size,
            rd: ex.rd,
            alu: ex.alu,
            load_data: ld,
            ctrl: ex.ctrl,
            trap: trap.clone(),
        });

        if trap.is_some() {
            flush_remaining = true;
        }
    }

    cpu.mem_wb.entries = mem_results;
    cpu.ex_mem_shadow = ex_entries;
}
