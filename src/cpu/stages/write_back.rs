use crate::cpu::Cpu;

pub fn wb_stage(cpu: &mut Cpu) -> Result<(), String> {
    let wb = cpu.mem_wb.clone();

    if let Some(trap_msg) = wb.trap {
        return Err(trap_msg);
    }

    if cpu.trace {
        eprintln!("WB  pc={:#x} inst={:#010x}", wb.pc, wb.inst);
    }

    if wb.inst != 0x0000_0000 && wb.inst != 0x0000_0013 {
        cpu.stats.instructions_retired += 1;

        if wb.ctrl.mem_read {
            cpu.stats.inst_load += 1;
        } else if wb.ctrl.mem_write {
            cpu.stats.inst_store += 1;
        } else if wb.ctrl.branch || wb.ctrl.jump {
            cpu.stats.inst_branch += 1;
        } else if wb.ctrl.is_system {
            cpu.stats.inst_system += 1;
        } else {
            cpu.stats.inst_alu += 1;
        }
    }

    if wb.ctrl.reg_write && wb.rd != 0 {
        let val = if wb.ctrl.mem_read {
            wb.load_data
        } else if wb.ctrl.jump {
            wb.pc.wrapping_add(4)
        } else {
            wb.alu
        };
        cpu.regs.write(wb.rd, val);
    }

    Ok(())
}
