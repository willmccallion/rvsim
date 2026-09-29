//! Instruction Disassembler Unit Tests.
//!
//! Verifies that the disassembler correctly converts common instruction
//! encodings to human-readable mnemonics for RV64I, RV64M, RV64A,
//! RV64F/D, and privileged instructions.

use rvsim_core::isa::disasm::disassemble;

#[test]
fn disasm_add() {
    let inst: u32 = 0x00C5_8533; // add a0, a1, a2
    let text = disassemble(inst);
    assert!(text.starts_with("add "), "Expected 'add', got '{}'", text);
    assert!(text.contains("a0"), "Expected a0 (x10) in '{}'", text);
}

#[test]
fn disasm_sub() {
    let inst: u32 = 0x40C5_8533;
    let text = disassemble(inst);
    assert!(text.starts_with("sub "), "Expected 'sub', got '{}'", text);
}

#[test]
fn disasm_addi() {
    let text = disassemble(0x00A0_0513);
    assert!(text.starts_with("addi "), "Expected 'addi', got '{}'", text);
    assert!(text.contains("10"), "Expected immediate 10 in '{}'", text);
}

#[test]
fn disasm_addi_negative() {
    let inst: u32 = 0xFFF0_0513;
    let text = disassemble(inst);
    assert!(text.starts_with("addi "), "Expected 'addi', got '{}'", text);
    assert!(text.contains("-1"), "Expected immediate -1 in '{}'", text);
}

#[test]
fn disasm_slli() {
    let inst: u32 = 0x0035_1513;
    let text = disassemble(inst);
    assert!(text.starts_with("slli "), "Expected 'slli', got '{}'", text);
    assert!(text.contains("3"), "Expected shamt 3 in '{}'", text);
}

#[test]
fn disasm_srli() {
    let inst: u32 = 0x0055_5513;
    let text = disassemble(inst);
    assert!(text.starts_with("srli "), "Expected 'srli', got '{}'", text);
}

#[test]
fn disasm_ld() {
    let inst: u32 = 0x0081_3503;
    let text = disassemble(inst);
    assert!(text.starts_with("ld "), "Expected 'ld', got '{}'", text);
}

#[test]
fn disasm_sd() {
    let inst: u32 = 0x00A1_3423;
    let text = disassemble(inst);
    assert!(text.starts_with("sd "), "Expected 'sd', got '{}'", text);
}

#[test]
fn disasm_lb() {
    let inst: u32 = 0x0005_8503;
    let text = disassemble(inst);
    assert!(text.starts_with("lb "), "Expected 'lb', got '{}'", text);
}

#[test]
fn disasm_beq() {
    let inst: u32 = 0x00B5_0063;
    let text = disassemble(inst);
    assert!(text.starts_with("beq "), "Expected 'beq', got '{}'", text);
}

#[test]
fn disasm_bne() {
    let inst: u32 = 0x0005_1063;
    let text = disassemble(inst);
    assert!(text.starts_with("bne "), "Expected 'bne', got '{}'", text);
}

#[test]
fn disasm_lui() {
    let inst: u32 = 0x1234_5537;
    let text = disassemble(inst);
    assert!(text.starts_with("lui "), "Expected 'lui', got '{}'", text);
    assert!(text.contains("0x12345"), "Expected 0x12345 in '{}'", text);
}

#[test]
fn disasm_auipc() {
    let inst: u32 = 0x0000_1517;
    let text = disassemble(inst);
    assert!(text.starts_with("auipc "), "Expected 'auipc', got '{}'", text);
}

#[test]
fn disasm_jal() {
    let inst: u32 = 0x0000_00EF;
    let text = disassemble(inst);
    assert!(text.starts_with("jal "), "Expected 'jal', got '{}'", text);
}

#[test]
fn disasm_jalr() {
    let inst: u32 = 0x0000_8067;
    let text = disassemble(inst);
    assert!(text.starts_with("jalr "), "Expected 'jalr', got '{}'", text);
}

#[test]
fn disasm_ecall() {
    let text = disassemble(0x0000_0073);
    assert_eq!(text, "ecall");
}

#[test]
fn disasm_ebreak() {
    let text = disassemble(0x0010_0073);
    assert_eq!(text, "ebreak");
}

#[test]
fn disasm_mret() {
    let text = disassemble(0x3020_0073);
    assert_eq!(text, "mret");
}

#[test]
fn disasm_sret() {
    let text = disassemble(0x1020_0073);
    assert_eq!(text, "sret");
}

#[test]
fn disasm_wfi() {
    let text = disassemble(0x1050_0073);
    assert_eq!(text, "wfi");
}

#[test]
fn disasm_fence() {
    let inst: u32 = 0x0FF0_000F;
    let text = disassemble(inst);
    assert_eq!(text, "fence");
}

#[test]
fn disasm_fence_i() {
    let inst: u32 = 0x0000_100F;
    let text = disassemble(inst);
    assert_eq!(text, "fence.i");
}

#[test]
fn disasm_csrrw() {
    let inst: u32 = 0x3005_9573;
    let text = disassemble(inst);
    assert!(text.starts_with("csrrw "), "Expected 'csrrw', got '{}'", text);
    assert!(text.contains("0x300"), "Expected CSR 0x300 in '{}'", text);
}

#[test]
fn disasm_csrrs() {
    let inst: u32 = 0x3005_A573;
    let text = disassemble(inst);
    assert!(text.starts_with("csrrs "), "Expected 'csrrs', got '{}'", text);
}

#[test]
fn disasm_mul() {
    let inst: u32 = 0x02C5_8533;
    let text = disassemble(inst);
    assert!(text.starts_with("mul "), "Expected 'mul', got '{}'", text);
}

#[test]
fn disasm_divu() {
    let inst: u32 = 0x02C5_D533;
    let text = disassemble(inst);
    assert!(text.starts_with("divu "), "Expected 'divu', got '{}'", text);
}

#[test]
fn disasm_addw() {
    let inst: u32 = 0x00C5_853B;
    let text = disassemble(inst);
    assert!(text.starts_with("addw "), "Expected 'addw', got '{}'", text);
}

#[test]
fn disasm_addiw() {
    let inst: u32 = 0x0055_851B;
    let text = disassemble(inst);
    assert!(text.starts_with("addiw "), "Expected 'addiw', got '{}'", text);
}

#[test]
fn disasm_unknown() {
    let text = disassemble(0x0000_0000);
    assert!(text.contains("unknown"), "Expected 'unknown' for all-zeroes, got '{}'", text);
}
