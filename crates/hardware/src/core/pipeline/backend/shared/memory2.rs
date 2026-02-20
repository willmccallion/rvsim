//! Memory2 Stage: D-cache access for loads, store buffer resolution.
//!
//! For loads: read data from the cache/memory (with store-to-load forwarding).
//! For stores: resolve the store buffer entry with paddr + data (NO memory write).
//! This stage is the same for both in-order and O3 backends.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::Cpu;
use crate::core::pipeline::latches::{Mem1Mem2Entry, Mem2WbEntry};
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::pipeline::store_buffer::{ForwardResult, StoreBuffer};
use crate::core::units::lsu::Lsu;

/// Executes the Memory2 stage: D-cache access + store buffer forwarding.
pub fn memory2_stage(
    cpu: &mut Cpu,
    input: &mut Vec<Mem1Mem2Entry>,
    output: &mut Vec<Mem2WbEntry>,
    store_buffer: &mut StoreBuffer,
    _rob: &mut Rob,
) {
    let entries = std::mem::take(input);
    output.clear();

    let mut flush_remaining = false;

    for mem in entries {
        if flush_remaining {
            break;
        }

        // Propagate traps
        if let Some(ref trap) = mem.trap {
            if cpu.trace {
                eprintln!("M2  pc={:#x} # TRAP: {:?}", mem.pc, trap);
            }
            output.push(Mem2WbEntry {
                rob_tag: mem.rob_tag,
                pc: mem.pc,
                inst: mem.inst,
                inst_size: mem.inst_size,
                rd: mem.rd,
                alu: mem.alu,
                load_data: 0,
                ctrl: mem.ctrl,
                trap: mem.trap,
                exception_stage: mem.exception_stage,
            });
            flush_remaining = true;
            continue;
        }

        let raw_paddr = mem.paddr;
        let is_ram = raw_paddr >= cpu.ram_start && raw_paddr < cpu.ram_end;
        let ram_offset = if is_ram {
            (raw_paddr - cpu.ram_start) as usize
        } else {
            0
        };

        let mut ld: u64 = 0;
        let trap: Option<Trap> = None;
        let exception_stage: Option<ExceptionStage> = None;

        if mem.ctrl.atomic_op != AtomicOp::None {
            // Atomic operations
            match mem.ctrl.atomic_op {
                AtomicOp::Lr => {
                    ld = match store_buffer.forward_load(raw_paddr, mem.ctrl.width) {
                        ForwardResult::Hit(fwd) => fwd,
                        ForwardResult::Stall => {
                            input.push(mem);
                            break;
                        }
                        ForwardResult::Miss => match mem.ctrl.width {
                            MemWidth::Word => {
                                (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                            }
                            MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                            _ => 0,
                        },
                    };
                    cpu.set_reservation(raw_paddr);
                }
                AtomicOp::Sc => {
                    if cpu.check_reservation(raw_paddr) {
                        // SC success — store will be deferred to commit via store buffer
                        // Resolve the store buffer entry
                        store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, mem.store_data);
                        ld = 0; // success
                        cpu.clear_reservation();
                    } else {
                        // SC failed — cancel the store buffer entry (no memory write)
                        store_buffer.cancel(mem.rob_tag);
                        ld = 1; // fail — do NOT clear reservation
                    }
                }
                _ => {
                    // AMO: read old value (check store buffer first for forwarding)
                    let old_val = match store_buffer.forward_load(raw_paddr, mem.ctrl.width) {
                        ForwardResult::Hit(fwd) => fwd,
                        ForwardResult::Stall => {
                            input.push(mem);
                            break;
                        }
                        ForwardResult::Miss => match mem.ctrl.width {
                            MemWidth::Word => {
                                (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                            }
                            MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                            _ => 0,
                        },
                    };

                    let new_val = Lsu::atomic_alu(
                        mem.ctrl.atomic_op,
                        old_val,
                        mem.store_data,
                        mem.ctrl.width,
                    );

                    // Resolve store buffer with the computed new value
                    store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, new_val);

                    ld = old_val;
                    if cpu.check_reservation(raw_paddr) {
                        cpu.clear_reservation();
                    }
                }
            }
        } else if mem.ctrl.mem_read {
            // Check store buffer for forwarding first
            match store_buffer.forward_load(raw_paddr, mem.ctrl.width) {
                ForwardResult::Hit(forwarded) => {
                    // Apply sign extension for signed loads (LB, LH, LW on RV64).
                    // The store buffer returns raw masked data without sign extension.
                    ld = if mem.ctrl.signed_load {
                        match mem.ctrl.width {
                            MemWidth::Byte => (forwarded as u8 as i8) as i64 as u64,
                            MemWidth::Half => (forwarded as u16 as i16) as i64 as u64,
                            MemWidth::Word => (forwarded as u32 as i32) as i64 as u64,
                            _ => forwarded,
                        }
                    } else {
                        forwarded
                    };
                    // NaN-boxing for FP loads forwarded from store buffer
                    if mem.ctrl.fp_reg_write && matches!(mem.ctrl.width, MemWidth::Word) {
                        ld |= 0xFFFF_FFFF_0000_0000;
                    }
                    if cpu.trace {
                        eprintln!(
                            "M2  pc={:#x} LOAD forwarded from store buffer: {:#x}",
                            mem.pc, ld
                        );
                    }
                }
                ForwardResult::Stall => {
                    // Partial overlap — push back and stall
                    input.push(mem);
                    break;
                }
                ForwardResult::Miss => {
                    // Read from memory/cache
                    ld = if is_ram {
                        unsafe {
                            match (mem.ctrl.width, mem.ctrl.signed_load) {
                                (MemWidth::Byte, true) => {
                                    (*cpu.ram_ptr.add(ram_offset) as i8) as i64 as u64
                                }
                                (MemWidth::Half, true) => {
                                    ((cpu.ram_ptr.add(ram_offset) as *const u16).read_unaligned()
                                        as i16) as i64 as u64
                                }
                                (MemWidth::Word, true) => {
                                    ((cpu.ram_ptr.add(ram_offset) as *const u32).read_unaligned()
                                        as i32) as i64 as u64
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
                        match (mem.ctrl.width, mem.ctrl.signed_load) {
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

                    // NaN-boxing for FP loads
                    if mem.ctrl.fp_reg_write && matches!(mem.ctrl.width, MemWidth::Word) {
                        ld |= 0xFFFF_FFFF_0000_0000;
                    }
                }
            }

            if cpu.trace {
                eprintln!(
                    "M2  pc={:#x} LOAD paddr={:#x} data={:#x}",
                    mem.pc, raw_paddr, ld
                );
            }
        } else if mem.ctrl.mem_write {
            // Stores: resolve store buffer with paddr + data, NO memory write
            store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, mem.store_data);

            if cpu.check_reservation(raw_paddr) {
                cpu.clear_reservation();
            }

            if cpu.trace {
                eprintln!(
                    "M2  pc={:#x} STORE resolved paddr={:#x} data={:#x}",
                    mem.pc, raw_paddr, mem.store_data
                );
            }
        } else if cpu.trace {
            eprintln!("M2  pc={:#x} (pass-through)", mem.pc);
        }

        output.push(Mem2WbEntry {
            rob_tag: mem.rob_tag,
            pc: mem.pc,
            inst: mem.inst,
            inst_size: mem.inst_size,
            rd: mem.rd,
            alu: mem.alu,
            load_data: ld,
            ctrl: mem.ctrl,
            trap: trap.clone(),
            exception_stage,
        });

        if trap.is_some() {
            flush_remaining = true;
        }
    }
}
