use crate::cpu::pipeline::IFID;
use crate::cpu::{AccessType, Cpu};
use crate::isa::opcodes;

pub fn fetch_stage(cpu: &mut Cpu) -> Result<(), String> {
    let pc = cpu.pc;

    let (paddr, tlb_latency, fault) = cpu.translate(pc, AccessType::Execute);
    cpu.stall_cycles += tlb_latency;

    if let Some(trap_msg) = fault {
        return Err(trap_msg);
    }

    let latency = cpu.simulate_memory_access(paddr, true, false);
    cpu.stall_cycles += latency;

    let inst = cpu.read_inst(paddr);

    if cpu.trace {
        eprintln!("IF  pc={:#x} (phys={:#x}) inst={:#010x}", pc, paddr, inst);
    }

    cpu.if_id = IFID { pc, inst };

    let opcode = inst & 0x7f;
    let rd = ((inst >> 7) & 0x1f) as u32;
    let rs1 = ((inst >> 15) & 0x1f) as u32;

    let mut next_pc = pc.wrapping_add(4);

    if opcode == opcodes::OP_BRANCH {
        let (pred_taken, pred_target) = cpu.branch_predictor.predict_branch(pc);
        if pred_taken {
            if let Some(tgt) = pred_target {
                next_pc = tgt;
            }
        }
    } else if opcode == opcodes::OP_JAL {
        if let Some(tgt) = cpu.branch_predictor.predict_btb(pc) {
            next_pc = tgt;
        }
    } else if opcode == opcodes::OP_JALR {
        if rd == 0 && rs1 == 1 {
            if let Some(tgt) = cpu.branch_predictor.predict_return() {
                next_pc = tgt;
            }
        } else if let Some(tgt) = cpu.branch_predictor.predict_btb(pc) {
            next_pc = tgt;
        }
    }

    cpu.pc = next_pc;
    Ok(())
}
