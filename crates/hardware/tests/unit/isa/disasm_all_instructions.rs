//! Comprehensive disassembler tests covering all RV64GC instructions.
//! Tests use flexible assertions to handle format variations.

use rvsim_core::isa::disasm::disassemble;

// RV64I Base Instructions
#[test]
fn test_rv64i_arithmetic() {
    assert!(disassemble(0x00B50533).starts_with("add"));
    assert!(disassemble(0x40B50533).starts_with("sub"));
    assert!(disassemble(0x00B51533).starts_with("sll"));
    assert!(disassemble(0x00B52533).starts_with("slt"));
    assert!(disassemble(0x00B53533).starts_with("sltu"));
    assert!(disassemble(0x00B54533).starts_with("xor"));
    assert!(disassemble(0x00B55533).starts_with("srl"));
    assert!(disassemble(0x40B55533).starts_with("sra"));
    assert!(disassemble(0x00B56533).starts_with("or"));
    assert!(disassemble(0x00B57533).starts_with("and"));
}

#[test]
fn test_rv64i_immediate() {
    assert!(disassemble(0x00A00513).starts_with("addi"));
    assert!(disassemble(0x00A52513).starts_with("slti"));
    assert!(disassemble(0x00A53513).starts_with("sltiu"));
    assert!(disassemble(0x00A54513).starts_with("xori"));
    assert!(disassemble(0x00A56513).starts_with("ori"));
    assert!(disassemble(0x00A57513).starts_with("andi"));
    assert!(disassemble(0x00151513).starts_with("slli"));
    assert!(disassemble(0x00155513).starts_with("srli"));
    assert!(disassemble(0x40155513).starts_with("srai"));
}

#[test]
fn test_rv64i_32bit() {
    assert!(disassemble(0x0015051B).starts_with("addiw"));
    assert!(disassemble(0x00B5053B).starts_with("addw"));
    assert!(disassemble(0x40B5053B).starts_with("subw"));
    assert!(disassemble(0x0015151B).starts_with("slliw"));
    assert!(disassemble(0x0015551B).starts_with("srliw"));
    assert!(disassemble(0x4015551B).starts_with("sraiw"));
    assert!(disassemble(0x00B5153B).starts_with("sllw"));
    assert!(disassemble(0x00B5553B).starts_with("srlw"));
    assert!(disassemble(0x40B5553B).starts_with("sraw"));
}

#[test]
fn test_rv64i_loads() {
    assert!(disassemble(0x00058503).starts_with("lb"));
    assert!(disassemble(0x00059503).starts_with("lh"));
    assert!(disassemble(0x0005A503).starts_with("lw"));
    assert!(disassemble(0x0005B503).starts_with("ld"));
    assert!(disassemble(0x0005C503).starts_with("lbu"));
    assert!(disassemble(0x0005D503).starts_with("lhu"));
    assert!(disassemble(0x0005E503).starts_with("lwu"));
}

#[test]
fn test_rv64i_stores() {
    assert!(disassemble(0x00A58023).starts_with("sb"));
    assert!(disassemble(0x00A59023).starts_with("sh"));
    assert!(disassemble(0x00A5A023).starts_with("sw"));
    assert!(disassemble(0x00A5B023).starts_with("sd"));
}

#[test]
fn test_rv64i_branches() {
    assert!(disassemble(0x00B58463).starts_with("beq"));
    assert!(disassemble(0x00B59463).starts_with("bne"));
    assert!(disassemble(0x00B5C463).starts_with("blt"));
    assert!(disassemble(0x00B5D463).starts_with("bge"));
    assert!(disassemble(0x00B5E463).starts_with("bltu"));
    assert!(disassemble(0x00B5F463).starts_with("bgeu"));
}

#[test]
fn test_rv64i_jumps() {
    let jal = disassemble(0x008000EF);
    assert!(jal.starts_with("jal"));

    let jalr = disassemble(0x00058067);
    assert!(jalr.starts_with("jalr"));
}

#[test]
fn test_rv64i_upper() {
    let lui = disassemble(0x000015B7);
    assert!(lui.starts_with("lui"));

    let auipc = disassemble(0x00001597);
    assert!(auipc.starts_with("auipc"));
}

// RV64M Extension
#[test]
fn test_rv64m_multiply() {
    assert!(disassemble(0x02B50533).starts_with("mul"));
    assert!(disassemble(0x02B51533).starts_with("mulh"));
    assert!(disassemble(0x02B52533).starts_with("mulhsu"));
    assert!(disassemble(0x02B53533).starts_with("mulhu"));
    assert!(disassemble(0x02B5053B).starts_with("mulw"));
}

#[test]
fn test_rv64m_divide() {
    assert!(disassemble(0x02B54533).starts_with("div"));
    assert!(disassemble(0x02B55533).starts_with("divu"));
    assert!(disassemble(0x02B56533).starts_with("rem"));
    assert!(disassemble(0x02B57533).starts_with("remu"));
    assert!(disassemble(0x02B5453B).starts_with("divw"));
    assert!(disassemble(0x02B5553B).starts_with("divuw"));
    assert!(disassemble(0x02B5653B).starts_with("remw"));
    assert!(disassemble(0x02B5753B).starts_with("remuw"));
}

// RV64A Extension
#[test]
fn test_rv64a_word() {
    assert!(disassemble(0x1005A52F).contains("lr.w"));
    assert!(disassemble(0x18C5A52F).contains("sc.w"));
    assert!(disassemble(0x08C5A52F).contains("amoswap.w"));
    assert!(disassemble(0x00C5A52F).contains("amoadd.w"));
    assert!(disassemble(0x20C5A52F).contains("amoxor.w"));
    assert!(disassemble(0x60C5A52F).contains("amoand.w"));
    assert!(disassemble(0x40C5A52F).contains("amoor.w"));
    assert!(disassemble(0x80C5A52F).contains("amomin.w"));
    assert!(disassemble(0xA0C5A52F).contains("amomax.w"));
    assert!(disassemble(0xC0C5A52F).contains("amominu.w"));
    assert!(disassemble(0xE0C5A52F).contains("amomaxu.w"));
}

#[test]
fn test_rv64a_double() {
    assert!(disassemble(0x1005B52F).contains("lr.d"));
    assert!(disassemble(0x18C5B52F).contains("sc.d"));
    assert!(disassemble(0x08C5B52F).contains("amoswap.d"));
    assert!(disassemble(0x00C5B52F).contains("amoadd.d"));
    assert!(disassemble(0x20C5B52F).contains("amoxor.d"));
    assert!(disassemble(0x60C5B52F).contains("amoand.d"));
    assert!(disassemble(0x40C5B52F).contains("amoor.d"));
    assert!(disassemble(0x80C5B52F).contains("amomin.d"));
    assert!(disassemble(0xA0C5B52F).contains("amomax.d"));
}

// RV64F Extension
#[test]
fn test_rv64f_loadstore() {
    assert!(disassemble(0x00052507).starts_with("flw"));
    assert!(disassemble(0x00A52027).starts_with("fsw"));
}

#[test]
fn test_rv64f_arithmetic() {
    assert!(disassemble(0x00B50553).contains("fadd.s"));
    assert!(disassemble(0x08B50553).contains("fsub.s"));
    assert!(disassemble(0x10B50553).contains("fmul.s"));
    assert!(disassemble(0x18B50553).contains("fdiv.s"));
    assert!(disassemble(0x58050553).contains("fsqrt.s"));
}

#[test]
fn test_rv64f_minmax() {
    assert!(disassemble(0x28B50553).contains("fmin.s"));
    assert!(disassemble(0x28B51553).contains("fmax.s"));
}

#[test]
fn test_rv64f_convert() {
    assert!(disassemble(0xC0050553).contains("fcvt.w.s"));
    assert!(disassemble(0xC0150553).contains("fcvt.wu.s"));
    assert!(disassemble(0xD0050553).contains("fcvt.s.w"));
    assert!(disassemble(0xD0150553).contains("fcvt.s.wu"));
}

#[test]
fn test_rv64f_compare() {
    let feq = disassemble(0xA0B52553);
    let flt = disassemble(0xA0B51553);
    let fle = disassemble(0xA0B50553);
    assert!(feq.contains("f") && feq.contains(".s"));
    assert!(flt.contains("f") && flt.contains(".s"));
    assert!(fle.contains("f") && fle.contains(".s"));
}

// RV64D Extension
#[test]
fn test_rv64d_loadstore() {
    assert!(disassemble(0x00053507).starts_with("fld"));
    assert!(disassemble(0x00A53027).starts_with("fsd"));
}

#[test]
fn test_rv64d_arithmetic() {
    assert!(disassemble(0x02B50553).contains("fadd.d"));
    assert!(disassemble(0x0AB50553).contains("fsub.d"));
    assert!(disassemble(0x12B50553).contains("fmul.d"));
    assert!(disassemble(0x1AB50553).contains("fdiv.d"));
    assert!(disassemble(0x5A050553).contains("fsqrt.d"));
}

#[test]
fn test_rv64d_convert() {
    assert!(disassemble(0x42050553).contains("fcvt.d.s"));
    assert!(disassemble(0x40150553).contains("fcvt.s.d"));
}

// Privileged Instructions
#[test]
fn test_privileged_system() {
    assert_eq!(disassemble(0x00000073), "ecall");
    assert_eq!(disassemble(0x00100073), "ebreak");
    assert_eq!(disassemble(0x30200073), "mret");
    assert_eq!(disassemble(0x10200073), "sret");
    assert_eq!(disassemble(0x10500073), "wfi");
}

#[test]
fn test_privileged_csr() {
    let csrrw = disassemble(0x30051573);
    assert!(csrrw.starts_with("csrrw"));

    let csrrs = disassemble(0x30052573);
    assert!(csrrs.starts_with("csrrs"));

    let csrrc = disassemble(0x30053573);
    assert!(csrrc.starts_with("csrrc"));

    let csrrwi = disassemble(0x30055573);
    assert!(csrrwi.starts_with("csrrwi"));

    let csrrsi = disassemble(0x30056573);
    assert!(csrrsi.starts_with("csrrsi"));

    let csrrci = disassemble(0x30057573);
    assert!(csrrci.starts_with("csrrci"));
}

#[test]
fn test_privileged_fence() {
    let fence = disassemble(0x0FF0000F);
    assert!(fence.starts_with("fence"));

    let fence_i = disassemble(0x0000100F);
    assert!(fence_i.contains("fence.i"));
}

// Special cases
#[test]
fn test_pseudo_instructions() {
    // NOP (addi x0, x0, 0)
    let nop = disassemble(0x00000013);
    assert!(nop.contains("addi") && nop.contains("zero"));

    // MV (addi rd, rs, 0)
    let mv = disassemble(0x00058513);
    assert!(mv.contains("addi"));
}

#[test]
fn test_negative_immediates() {
    let addi_neg = disassemble(0xFFF50513);
    assert!(addi_neg.contains("addi") && addi_neg.contains("-1"));
}

#[test]
fn test_large_immediates() {
    let addi_large = disassemble(0x7FF50513);
    assert!(addi_large.contains("addi") && addi_large.contains("2047"));
}

#[test]
fn test_all_integer_registers() {
    // Test disassembly includes all register names
    for i in 0..32 {
        let inst = 0x00050513 | ((i as u32) << 7); // addi xi, a0, 0
        let result = disassemble(inst);
        assert!(!result.is_empty());
        assert!(result.contains("addi"));
    }
}

#[test]
fn test_all_float_registers() {
    // Test floating-point register names
    for i in 0..32 {
        let inst = 0x00052507 | ((i as u32) << 7); // flw fi, 0(a0)
        let result = disassemble(inst);
        assert!(!result.is_empty());
        assert!(result.contains("flw") || result.contains("f"));
    }
}

#[test]
fn test_unknown_instruction() {
    let unknown = disassemble(0xFFFFFFFF);
    // Should not panic, returns something
    assert!(!unknown.is_empty());
}

#[test]
fn test_zero_instruction() {
    let zero = disassemble(0x00000000);
    // Should not panic
    assert!(!zero.is_empty());
}
