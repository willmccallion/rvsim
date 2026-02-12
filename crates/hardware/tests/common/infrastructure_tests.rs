use super::builder::instruction::InstructionBuilder;
use super::builder::pipeline_state::{ExMemBuilder, IdExBuilder, IfIdBuilder, MemWbBuilder};
use super::harness::TestContext;
use super::mocks::interrupts::MockInterruptController;
use super::mocks::memory::MockMemory;
use riscv_core::core::pipeline::signals::ControlSignals;
use riscv_core::isa::rv64i::opcodes::*;

// ─── InstructionBuilder: R-type encoding ───────────────────────────────────

#[test]
fn builder_add_encodes_r_type() {
    let inst = InstructionBuilder::new().add(1, 2, 3).build();
    assert_eq!(inst & 0x7F, OP_REG);
    assert_eq!((inst >> 7) & 0x1F, 1); // rd = x1
    assert_eq!((inst >> 12) & 0x7, 0); // funct3 = 000
    assert_eq!((inst >> 15) & 0x1F, 2); // rs1 = x2
    assert_eq!((inst >> 20) & 0x1F, 3); // rs2 = x3
    assert_eq!((inst >> 25) & 0x7F, 0); // funct7 = 0000000
}

#[test]
fn builder_sub_sets_funct7() {
    let inst = InstructionBuilder::new().sub(1, 2, 3).build();
    assert_eq!(inst & 0x7F, OP_REG);
    assert_eq!((inst >> 25) & 0x7F, 0b0100000);
}

#[test]
fn builder_and_or_xor_funct3() {
    let and_inst = InstructionBuilder::new().and(1, 2, 3).build();
    assert_eq!((and_inst >> 12) & 0x7, 0b111);

    let or_inst = InstructionBuilder::new().or(1, 2, 3).build();
    assert_eq!((or_inst >> 12) & 0x7, 0b110);

    let xor_inst = InstructionBuilder::new().xor(1, 2, 3).build();
    assert_eq!((xor_inst >> 12) & 0x7, 0b100);
}

#[test]
fn builder_sll_srl_sra_funct3_and_funct7() {
    let sll = InstructionBuilder::new().sll(1, 2, 3).build();
    assert_eq!((sll >> 12) & 0x7, 0b001);
    assert_eq!((sll >> 25) & 0x7F, 0b0000000);

    let srl = InstructionBuilder::new().srl(1, 2, 3).build();
    assert_eq!((srl >> 12) & 0x7, 0b101);
    assert_eq!((srl >> 25) & 0x7F, 0b0000000);

    let sra = InstructionBuilder::new().sra(1, 2, 3).build();
    assert_eq!((sra >> 12) & 0x7, 0b101);
    assert_eq!((sra >> 25) & 0x7F, 0b0100000);
}

#[test]
fn builder_slt_sltu() {
    let slt = InstructionBuilder::new().slt(1, 2, 3).build();
    assert_eq!((slt >> 12) & 0x7, 0b010);

    let sltu = InstructionBuilder::new().sltu(1, 2, 3).build();
    assert_eq!((sltu >> 12) & 0x7, 0b011);
}

// ─── InstructionBuilder: I-type encoding ───────────────────────────────────

#[test]
fn builder_addi_encodes_i_type() {
    let inst = InstructionBuilder::new().addi(1, 0, 42).build();
    assert_eq!(inst & 0x7F, OP_IMM);
    assert_eq!((inst >> 7) & 0x1F, 1); // rd = x1
    assert_eq!((inst >> 12) & 0x7, 0); // funct3 = 000
    assert_eq!((inst >> 15) & 0x1F, 0); // rs1 = x0
    let imm = (inst >> 20) & 0xFFF;
    assert_eq!(imm, 42);
}

#[test]
fn builder_addi_negative_immediate() {
    let inst = InstructionBuilder::new().addi(1, 0, -1).build();
    let imm = (inst >> 20) & 0xFFF;
    assert_eq!(
        imm, 0xFFF,
        "Negative immediate -1 should be encoded as 0xFFF (12-bit)"
    );
}

#[test]
fn builder_immediate_variants() {
    let andi = InstructionBuilder::new().andi(5, 6, 0xFF).build();
    assert_eq!(andi & 0x7F, OP_IMM);
    assert_eq!((andi >> 12) & 0x7, 0b111);

    let ori = InstructionBuilder::new().ori(5, 6, 0xFF).build();
    assert_eq!((ori >> 12) & 0x7, 0b110);

    let xori = InstructionBuilder::new().xori(5, 6, 0xFF).build();
    assert_eq!((xori >> 12) & 0x7, 0b100);

    let slti = InstructionBuilder::new().slti(5, 6, 10).build();
    assert_eq!((slti >> 12) & 0x7, 0b010);

    let sltiu = InstructionBuilder::new().sltiu(5, 6, 10).build();
    assert_eq!((sltiu >> 12) & 0x7, 0b011);
}

// ─── InstructionBuilder: Load/Store encoding ───────────────────────────────

#[test]
fn builder_lw_encodes_i_type_load() {
    let inst = InstructionBuilder::new().lw(1, 2, 8).build();
    assert_eq!(inst & 0x7F, OP_LOAD);
    assert_eq!((inst >> 12) & 0x7, 0b010); // funct3 = LW
    assert_eq!((inst >> 20) & 0xFFF, 8);
}

#[test]
fn builder_ld_encodes_doubleword_load() {
    let inst = InstructionBuilder::new().ld(1, 2, 16).build();
    assert_eq!(inst & 0x7F, OP_LOAD);
    assert_eq!((inst >> 12) & 0x7, 0b011); // funct3 = LD
}

#[test]
fn builder_sw_encodes_s_type() {
    let inst = InstructionBuilder::new().sw(2, 3, 32).build();
    assert_eq!(inst & 0x7F, OP_STORE);
    assert_eq!((inst >> 12) & 0x7, 0b010); // funct3 = SW
    assert_eq!((inst >> 15) & 0x1F, 2); // rs1 = x2
    assert_eq!((inst >> 20) & 0x1F, 3); // rs2 = x3
    // Reconstruct S-type immediate: imm[4:0] = bits[11:7], imm[11:5] = bits[31:25]
    let imm_4_0 = (inst >> 7) & 0x1F;
    let imm_11_5 = (inst >> 25) & 0x7F;
    let imm = (imm_11_5 << 5) | imm_4_0;
    assert_eq!(imm, 32);
}

#[test]
fn builder_sd_encodes_doubleword_store() {
    let inst = InstructionBuilder::new().sd(2, 3, 8).build();
    assert_eq!(inst & 0x7F, OP_STORE);
    assert_eq!((inst >> 12) & 0x7, 0b011); // funct3 = SD
}

// ─── InstructionBuilder: Branch encoding (B-type) ─────────────────────────

#[test]
fn builder_beq_encodes_b_type() {
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    assert_eq!(inst & 0x7F, OP_BRANCH);
    assert_eq!((inst >> 12) & 0x7, 0b000); // funct3 = BEQ
    assert_eq!((inst >> 15) & 0x1F, 1); // rs1
    assert_eq!((inst >> 20) & 0x1F, 2); // rs2
}

#[test]
fn builder_branch_variants_funct3() {
    let bne = InstructionBuilder::new().bne(1, 2, 8).build();
    assert_eq!((bne >> 12) & 0x7, 0b001);

    let blt = InstructionBuilder::new().blt(1, 2, 8).build();
    assert_eq!((blt >> 12) & 0x7, 0b100);

    let bge = InstructionBuilder::new().bge(1, 2, 8).build();
    assert_eq!((bge >> 12) & 0x7, 0b101);

    let bltu = InstructionBuilder::new().bltu(1, 2, 8).build();
    assert_eq!((bltu >> 12) & 0x7, 0b110);

    let bgeu = InstructionBuilder::new().bgeu(1, 2, 8).build();
    assert_eq!((bgeu >> 12) & 0x7, 0b111);
}

// ─── InstructionBuilder: U-type encoding ───────────────────────────────────

#[test]
fn builder_lui_encodes_u_type() {
    let inst = InstructionBuilder::new().lui(1, 0x12345).build();
    assert_eq!(inst & 0x7F, OP_LUI);
    assert_eq!((inst >> 7) & 0x1F, 1);
    let upper = inst >> 12;
    assert_eq!(upper, 0x12345);
}

#[test]
fn builder_auipc_encodes_u_type() {
    let inst = InstructionBuilder::new().auipc(1, 0x1).build();
    assert_eq!(inst & 0x7F, OP_AUIPC);
    assert_eq!((inst >> 7) & 0x1F, 1);
}

// ─── InstructionBuilder: J-type encoding ───────────────────────────────────

#[test]
fn builder_jal_encodes_j_type() {
    let inst = InstructionBuilder::new().jal(1, 0).build();
    assert_eq!(inst & 0x7F, OP_JAL);
    assert_eq!((inst >> 7) & 0x1F, 1);
}

#[test]
fn builder_jalr_encodes_i_type() {
    let inst = InstructionBuilder::new().jalr(1, 2, 0).build();
    assert_eq!(inst & 0x7F, OP_JALR);
    assert_eq!((inst >> 7) & 0x1F, 1);
    assert_eq!((inst >> 15) & 0x1F, 2);
}

// ─── InstructionBuilder: RV64I 32-bit variants ────────────────────────────

#[test]
fn builder_addiw_encodes_op_imm_32() {
    let inst = InstructionBuilder::new().addiw(1, 2, 5).build();
    assert_eq!(inst & 0x7F, OP_IMM_32);
    assert_eq!((inst >> 12) & 0x7, 0b000);
}

#[test]
fn builder_addw_subw_encodes_op_reg_32() {
    let addw = InstructionBuilder::new().addw(1, 2, 3).build();
    assert_eq!(addw & 0x7F, OP_REG_32);
    assert_eq!((addw >> 25) & 0x7F, 0b0000000);

    let subw = InstructionBuilder::new().subw(1, 2, 3).build();
    assert_eq!(subw & 0x7F, OP_REG_32);
    assert_eq!((subw >> 25) & 0x7F, 0b0100000);
}

// ─── InstructionBuilder: NOP ───────────────────────────────────────────────

#[test]
fn builder_nop_is_addi_x0_x0_0() {
    let nop = InstructionBuilder::new().nop().build();
    let addi_x0 = InstructionBuilder::new().addi(0, 0, 0).build();
    assert_eq!(nop, addi_x0);
}

// ─── InstructionBuilder: raw field API ─────────────────────────────────────

#[test]
fn builder_raw_field_api() {
    // Build an ADD x1, x2, x3 using the raw field API
    let inst = InstructionBuilder::new()
        .opcode(OP_REG)
        .rd(1)
        .rs1(2)
        .rs2(3)
        .funct3(0b000)
        .funct7(0b0000000)
        .build();
    let via_helper = InstructionBuilder::new().add(1, 2, 3).build();
    assert_eq!(
        inst, via_helper,
        "Raw field API should produce same encoding as helper"
    );
}

// ─── Pipeline State Builders ───────────────────────────────────────────────

#[test]
fn ifid_builder_defaults_and_setters() {
    let entry = IfIdBuilder::new()
        .pc(0x1000)
        .inst(0xDEADBEEF)
        .predicted(0x2000)
        .build();
    assert_eq!(entry.pc, 0x1000);
    assert_eq!(entry.inst, 0xDEADBEEF);
    assert!(entry.pred_taken);
    assert_eq!(entry.pred_target, 0x2000);
}

#[test]
fn ifid_builder_defaults_are_zero() {
    let entry = IfIdBuilder::new().build();
    assert_eq!(entry.pc, 0);
    assert_eq!(entry.inst, 0);
    assert!(!entry.pred_taken);
    assert_eq!(entry.pred_target, 0);
}

#[test]
fn idex_builder_full_chain() {
    let ctrl = ControlSignals {
        reg_write: true,
        ..Default::default()
    };
    let entry = IdExBuilder::new()
        .pc(0x2000)
        .inst(0x12345678)
        .rs1(1, 100)
        .rs2(2, 200)
        .rd(3)
        .imm(42)
        .control(ctrl)
        .build();
    assert_eq!(entry.pc, 0x2000);
    assert_eq!(entry.inst, 0x12345678);
    assert_eq!(entry.rs1, 1);
    assert_eq!(entry.rv1, 100);
    assert_eq!(entry.rs2, 2);
    assert_eq!(entry.rv2, 200);
    assert_eq!(entry.rd, 3);
    assert_eq!(entry.imm, 42);
    assert!(entry.ctrl.reg_write);
}

#[test]
fn exmem_builder() {
    let entry = ExMemBuilder::new()
        .pc(0x3000)
        .alu_result(0xCAFE)
        .store_data(0xBEEF)
        .rd(5)
        .build();
    assert_eq!(entry.pc, 0x3000);
    assert_eq!(entry.alu, 0xCAFE);
    assert_eq!(entry.store_data, 0xBEEF);
    assert_eq!(entry.rd, 5);
}

#[test]
fn memwb_builder() {
    let entry = MemWbBuilder::new()
        .pc(0x4000)
        .rd(7)
        .alu_result(0x1234)
        .load_data(0x5678)
        .build();
    assert_eq!(entry.pc, 0x4000);
    assert_eq!(entry.rd, 7);
    assert_eq!(entry.alu, 0x1234);
    assert_eq!(entry.load_data, 0x5678);
}

// ─── TestContext (Harness) ─────────────────────────────────────────────────

#[test]
fn harness_boot_default_pc() {
    let ctx = TestContext::new();
    assert_eq!(ctx.cpu.pc, 0x8000_0000, "CPU should start at RAM base");
}

#[test]
fn harness_with_memory_adds_device() {
    let mut ctx = TestContext::new().with_memory(4096, 0x1000);
    ctx.cpu.bus.bus.write_u32(0x1000, 0xDEADBEEF);
    assert_eq!(ctx.cpu.bus.bus.read_u32(0x1000), 0xDEADBEEF);
}

#[test]
fn harness_load_program_writes_instructions_and_sets_pc() {
    let nop = InstructionBuilder::new().nop().build();
    let addi = InstructionBuilder::new().addi(1, 0, 42).build();
    let program = [nop, addi];

    let mut ctx = TestContext::new()
        .with_memory(4096, 0x1000)
        .load_program(0x1000, &program);

    assert_eq!(ctx.cpu.pc, 0x1000, "PC should be set to program base");
    assert_eq!(ctx.cpu.bus.bus.read_u32(0x1000), nop);
    assert_eq!(ctx.cpu.bus.bus.read_u32(0x1004), addi);
}

#[test]
fn harness_set_and_get_reg() {
    let mut ctx = TestContext::new();
    ctx.set_reg(5, 0xDEAD);
    assert_eq!(ctx.get_reg(5), 0xDEAD);
}

#[test]
fn harness_x0_always_zero() {
    let mut ctx = TestContext::new();
    ctx.set_reg(0, 999);
    assert_eq!(ctx.get_reg(0), 0, "x0 must always read as zero");
}

// ─── MockMemory ────────────────────────────────────────────────────────────

#[test]
fn mock_memory_read_write_all_widths() {
    use riscv_core::soc::devices::Device;

    let mut mem = MockMemory::new(1024, 0x0);
    mem.write_u8(0, 0xAB);
    assert_eq!(mem.read_u8(0), 0xAB);

    mem.write_u16(8, 0x1234);
    assert_eq!(mem.read_u16(8), 0x1234);

    mem.write_u32(16, 0xDEADBEEF);
    assert_eq!(mem.read_u32(16), 0xDEADBEEF);

    mem.write_u64(24, 0xCAFEBABE_12345678);
    assert_eq!(mem.read_u64(24), 0xCAFEBABE_12345678);
}

#[test]
fn mock_memory_out_of_bounds_reads_zero() {
    use riscv_core::soc::devices::Device;

    let mut mem = MockMemory::new(16, 0x0);
    assert_eq!(mem.read_u32(20), 0, "Out-of-bounds read should return 0");
    assert_eq!(mem.read_u64(20), 0);
}

#[test]
#[should_panic(expected = "Bus Error")]
fn mock_memory_fault_injection_panics() {
    use riscv_core::soc::devices::Device;

    let mut mem = MockMemory::new(1024, 0x1000);
    mem.inject_fault(0x1010);
    mem.read_u32(0x10); // offset 0x10 -> address 0x1010
}

#[test]
fn mock_memory_fault_only_affects_target_address() {
    use riscv_core::soc::devices::Device;

    let mut mem = MockMemory::new(1024, 0x1000);
    mem.inject_fault(0x1010);
    // Non-faulting address should work fine
    mem.write_u32(0x0, 42);
    assert_eq!(mem.read_u32(0x0), 42);
}

#[test]
fn mock_memory_address_range() {
    use riscv_core::soc::devices::Device;

    let mem = MockMemory::new(4096, 0x8000_0000);
    let (base, size) = mem.address_range();
    assert_eq!(base, 0x8000_0000);
    assert_eq!(size, 4096);
}

#[test]
fn mock_memory_name() {
    use riscv_core::soc::devices::Device;

    let mem = MockMemory::new(64, 0);
    assert_eq!(mem.name(), "MockMemory");
}

// ─── MockInterruptController ───────────────────────────────────────────────

#[test]
fn interrupt_controller_raise_and_claim() {
    let mut ic = MockInterruptController::new();
    ic.enable(3);
    ic.raise(3);
    assert!(ic.is_pending());
    assert_eq!(ic.claim(), Some(3));
    assert!(!ic.is_pending(), "After claim, IRQ should be cleared");
}

#[test]
fn interrupt_controller_disabled_irq_not_claimable() {
    let mut ic = MockInterruptController::new();
    ic.raise(5);
    // IRQ 5 is raised but not enabled
    assert!(!ic.is_pending());
    assert_eq!(ic.claim(), None);
}

#[test]
fn interrupt_controller_multiple_irqs_claims_lowest() {
    let mut ic = MockInterruptController::new();
    ic.enable(2);
    ic.enable(5);
    ic.raise(5);
    ic.raise(2);
    // Should claim lowest (highest priority) first
    assert_eq!(ic.claim(), Some(2));
    assert_eq!(ic.claim(), Some(5));
    assert_eq!(ic.claim(), None);
}

#[test]
fn interrupt_controller_clear() {
    let mut ic = MockInterruptController::new();
    ic.enable(1);
    ic.raise(1);
    ic.clear(1);
    assert!(!ic.is_pending());
    assert_eq!(ic.claim(), None);
}
