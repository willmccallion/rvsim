use crate::cpu::control::MemWidth;
use crate::cpu::pipeline::MEMWB;
use crate::cpu::{AccessType, Cpu};

pub fn mem_stage(cpu: &mut Cpu) -> Result<(), String> {
    let ex = cpu.ex_mem.clone();

    // Propagate trap if present
    if let Some(trap_msg) = ex.trap {
        cpu.mem_wb = MEMWB {
            pc: ex.pc,
            inst: ex.inst,
            rd: ex.rd,
            alu: 0,
            load_data: 0,
            ctrl: ex.ctrl,
            trap: Some(trap_msg),
        };
        return Ok(());
    }

    if cpu.trace {
        eprintln!("MEM pc={:#x} inst={:#010x}", ex.pc, ex.inst);
    }

    let vaddr = ex.alu;
    let paddr;
    let mut ld: u64 = 0;
    let mut trap_msg: Option<String> = None;

    // Simulate Memory Hierarchy access if reading or writing
    if ex.ctrl.mem_read || ex.ctrl.mem_write {
        // 1. MMU Translation
        let access_type = if ex.ctrl.mem_write {
            AccessType::Write
        } else {
            AccessType::Read
        };
        let (phys, tlb_lat, fault) = cpu.translate(vaddr, access_type);

        cpu.stall_cycles += tlb_lat;
        paddr = phys;

        if let Some(msg) = fault {
            trap_msg = Some(msg);
        } else {
            // 2. Cache Simulation (only if no fault)
            // Skip cache stats for IO regions (Disk/UART) to avoid skewing stats
            if paddr < 0x9000_0000 {
                // Access Data Cache Hierarchy (L1D -> L2 -> L3 -> RAM)
                let latency = cpu.simulate_memory_access(paddr, false, ex.ctrl.mem_write);
                cpu.stall_cycles += latency;
            }

            // 3. Physical Access
            if ex.ctrl.mem_read {
                if (crate::devices::VIRTUAL_DISK_SIZE_ADDRESS
                    ..crate::devices::VIRTUAL_DISK_SIZE_ADDRESS + 8)
                    .contains(&paddr)
                    && cpu.trace
                {
                    let w = match ex.ctrl.width {
                        MemWidth::Byte => "u8",
                        MemWidth::Half => "u16",
                        MemWidth::Word => "u32",
                        MemWidth::Double => "u64",
                        _ => "?",
                    };

                    let peek = match (ex.ctrl.width, ex.ctrl.signed_load) {
                        (MemWidth::Byte, _) => cpu.load_u8(paddr) as u64,
                        (MemWidth::Half, _) => cpu.load_u16(paddr) as u64,
                        (MemWidth::Word, _) => cpu.load_u32(paddr) as u64,
                        (MemWidth::Double, _) => cpu.load_u64(paddr),
                        _ => 0,
                    };
                    eprintln!(
                        "DISK_SIZE READ @pc={:#x} addr={:#x} width={} -> {:#x} ({})",
                        ex.pc, paddr, w, peek, peek
                    );
                }
                ld = match (ex.ctrl.width, ex.ctrl.signed_load) {
                    (MemWidth::Byte, true) => (cpu.load_u8(paddr) as i8) as i64 as u64,
                    (MemWidth::Half, true) => (cpu.load_u16(paddr) as i16) as i64 as u64,
                    (MemWidth::Word, true) => (cpu.load_u32(paddr) as i32) as i64 as u64,
                    (MemWidth::Byte, false) => cpu.load_u8(paddr) as u64,
                    (MemWidth::Half, false) => cpu.load_u16(paddr) as u64,
                    (MemWidth::Word, false) => cpu.load_u32(paddr) as u64,
                    (MemWidth::Double, _) => cpu.load_u64(paddr),
                    _ => 0,
                };
            } else if ex.ctrl.mem_write {
                match ex.ctrl.width {
                    MemWidth::Byte => cpu.store_u8(paddr, ex.store_data as u8),
                    MemWidth::Half => cpu.store_u16(paddr, ex.store_data as u16),
                    MemWidth::Word => cpu.store_u32(paddr, ex.store_data as u32),
                    MemWidth::Double => cpu.store_u64(paddr, ex.store_data as u64),
                    _ => {}
                }
            }
        }
    }

    cpu.mem_wb = MEMWB {
        pc: ex.pc,
        inst: ex.inst,
        rd: ex.rd,
        alu: ex.alu, // Keep virtual address for debug
        load_data: ld,
        ctrl: ex.ctrl,
        trap: trap_msg,
    };

    Ok(())
}
