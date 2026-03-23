//! Vector disassembler unit tests (RVV 1.0).

use rvsim_core::isa::disasm::disassemble;

// ── Helper: build a 32-bit instruction word from fields ─────────────────────

const OP_V: u32 = 0b1010111;
const OP_LOAD_FP: u32 = 0b0000111;
const OP_STORE_FP: u32 = 0b0100111;

/// Build a vector arithmetic instruction.
fn vec_arith(funct6: u32, vm: u32, vs2: u32, vs1: u32, funct3: u32, vd: u32) -> u32 {
    (funct6 << 26) | (vm << 25) | (vs2 << 20) | (vs1 << 15) | (funct3 << 12) | (vd << 7) | OP_V
}

/// Build a vector load instruction (unit-stride).
fn vec_load_unit(nf: u32, mew: u32, mop: u32, vm: u32, lumop: u32, rs1: u32, width: u32, vd: u32) -> u32 {
    (nf << 29) | (mew << 28) | (mop << 26) | (vm << 25) | (lumop << 20) | (rs1 << 15) | (width << 12) | (vd << 7) | OP_LOAD_FP
}

/// Build a vector store instruction (unit-stride).
fn vec_store_unit(nf: u32, mew: u32, mop: u32, vm: u32, sumop: u32, rs1: u32, width: u32, vs3: u32) -> u32 {
    (nf << 29) | (mew << 28) | (mop << 26) | (vm << 25) | (sumop << 20) | (rs1 << 15) | (width << 12) | (vs3 << 7) | OP_STORE_FP
}

/// Build a vector load with rs2/vs2 (strided or indexed).
fn vec_load_rs2(nf: u32, mew: u32, mop: u32, vm: u32, rs2: u32, rs1: u32, width: u32, vd: u32) -> u32 {
    (nf << 29) | (mew << 28) | (mop << 26) | (vm << 25) | (rs2 << 20) | (rs1 << 15) | (width << 12) | (vd << 7) | OP_LOAD_FP
}

/// Build a vector store with rs2/vs2 (strided or indexed).
fn vec_store_rs2(nf: u32, mew: u32, mop: u32, vm: u32, rs2: u32, rs1: u32, width: u32, vs3: u32) -> u32 {
    (nf << 29) | (mew << 28) | (mop << 26) | (vm << 25) | (rs2 << 20) | (rs1 << 15) | (width << 12) | (vs3 << 7) | OP_STORE_FP
}

// funct3 categories
const OPIVV: u32 = 0b000;
const OPFVV: u32 = 0b001;
const OPMVV: u32 = 0b010;
const OPIVI: u32 = 0b011;
const OPIVX: u32 = 0b100;
const OPFVF: u32 = 0b101;
const OPMVX: u32 = 0b110;
const OPCFG: u32 = 0b111;

// ══════════════════════════════════════════════════════════
// Configuration instructions
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_config() {
    // vsetvli a0, a1, e32, m4, ta, ma — bit31=0
    // zimm = e32(010 << 3) + m4(010) + ta(1<<6) + ma(1<<7) = 0b11_010_010 = 0xD2
    let zimm: u32 = 0xD2;
    let inst = (0 << 31) | (zimm << 20) | (11 << 15) | (OPCFG << 12) | (10 << 7) | OP_V;
    let text = disassemble(inst);
    assert!(text.starts_with("vsetvli"), "got '{text}'");
    assert!(text.contains("a0"), "expected a0 in '{text}'");
    assert!(text.contains("a1"), "expected a1 in '{text}'");
    assert!(text.contains("e32"), "expected e32 in '{text}'");
    assert!(text.contains("m4"), "expected m4 in '{text}'");
    assert!(text.contains("ta"), "expected ta in '{text}'");
    assert!(text.contains("ma"), "expected ma in '{text}'");

    // vsetivli a0, 16, e8, m1, tu, mu
    // zimm = e8(000<<3) + m1(000) + tu(0<<6) + mu(0<<7) = 0x00
    let zimm: u32 = 0x00;
    let inst = (3 << 30) | (zimm << 20) | (16 << 15) | (OPCFG << 12) | (10 << 7) | OP_V;
    let text = disassemble(inst);
    assert!(text.starts_with("vsetivli"), "got '{text}'");
    assert!(text.contains("16"), "expected 16 in '{text}'");

    // vsetvl a0, a1, a2 — bit31=1, bit30=0
    let inst = (1 << 31) | (0 << 30) | (12 << 20) | (11 << 15) | (OPCFG << 12) | (10 << 7) | OP_V;
    let text = disassemble(inst);
    assert!(text.starts_with("vsetvl "), "got '{text}'");
    assert!(text.contains("a0"), "expected a0 in '{text}'");
    assert!(text.contains("a1"), "expected a1 in '{text}'");
    assert!(text.contains("a2"), "expected a2 in '{text}'");
}

// ══════════════════════════════════════════════════════════
// Integer arithmetic
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_int_arith() {
    // vadd.vv v1, v2, v3, unmasked
    let inst = vec_arith(0b000000, 1, 2, 3, OPIVV, 1);
    assert!(disassemble(inst).starts_with("vadd.vv"), "{}", disassemble(inst));

    // vsub.vx v4, v5, x10, masked
    let inst = vec_arith(0b000010, 0, 5, 10, OPIVX, 4);
    let text = disassemble(inst);
    assert!(text.starts_with("vsub.vx"), "got '{text}'");
    assert!(text.contains("v0.t"), "expected mask suffix in '{text}'");

    // vrsub.vi v6, v7, 5
    let inst = vec_arith(0b000011, 1, 7, 5, OPIVI, 6);
    let text = disassemble(inst);
    assert!(text.starts_with("vrsub.vi"), "got '{text}'");
    assert!(text.contains("5"), "expected imm 5 in '{text}'");

    // vand, vor, vxor
    assert!(disassemble(vec_arith(0b001001, 1, 2, 3, OPIVV, 1)).starts_with("vand.vv"));
    assert!(disassemble(vec_arith(0b001010, 1, 2, 3, OPIVV, 1)).starts_with("vor.vv"));
    assert!(disassemble(vec_arith(0b001011, 1, 2, 3, OPIVV, 1)).starts_with("vxor.vv"));

    // shifts
    assert!(disassemble(vec_arith(0b100101, 1, 2, 3, OPIVV, 1)).starts_with("vsll.vv"));
    assert!(disassemble(vec_arith(0b101000, 1, 2, 3, OPIVV, 1)).starts_with("vsrl.vv"));
    assert!(disassemble(vec_arith(0b101001, 1, 2, 3, OPIVV, 1)).starts_with("vsra.vv"));

    // min/max
    assert!(disassemble(vec_arith(0b000100, 1, 2, 3, OPIVV, 1)).starts_with("vminu.vv"));
    assert!(disassemble(vec_arith(0b000101, 1, 2, 3, OPIVV, 1)).starts_with("vmin.vv"));
    assert!(disassemble(vec_arith(0b000110, 1, 2, 3, OPIVV, 1)).starts_with("vmaxu.vv"));
    assert!(disassemble(vec_arith(0b000111, 1, 2, 3, OPIVV, 1)).starts_with("vmax.vv"));
}

// ══════════════════════════════════════════════════════════
// Integer multiply/divide
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_int_mul_div() {
    assert!(disassemble(vec_arith(0b100101, 1, 2, 3, OPMVV, 1)).starts_with("vmul.vv"));
    assert!(disassemble(vec_arith(0b100111, 1, 2, 3, OPMVV, 1)).starts_with("vmulh.vv"));
    assert!(disassemble(vec_arith(0b100100, 1, 2, 3, OPMVV, 1)).starts_with("vmulhu.vv"));
    assert!(disassemble(vec_arith(0b100110, 1, 2, 3, OPMVV, 1)).starts_with("vmulhsu.vv"));

    assert!(disassemble(vec_arith(0b100001, 1, 2, 3, OPMVV, 1)).starts_with("vdiv.vv"));
    assert!(disassemble(vec_arith(0b100000, 1, 2, 3, OPMVV, 1)).starts_with("vdivu.vv"));
    assert!(disassemble(vec_arith(0b100011, 1, 2, 3, OPMVV, 1)).starts_with("vrem.vv"));
    assert!(disassemble(vec_arith(0b100010, 1, 2, 3, OPMVV, 1)).starts_with("vremu.vv"));

    // multiply-add
    assert!(disassemble(vec_arith(0b101101, 1, 2, 3, OPMVV, 1)).starts_with("vmacc.vv"));
    assert!(disassemble(vec_arith(0b101001, 1, 2, 3, OPMVV, 1)).starts_with("vmadd.vv"));
    assert!(disassemble(vec_arith(0b101111, 1, 2, 3, OPMVV, 1)).starts_with("vnmsac.vv"));
    assert!(disassemble(vec_arith(0b101011, 1, 2, 3, OPMVV, 1)).starts_with("vnmsub.vv"));

    // .vx variants
    assert!(disassemble(vec_arith(0b100101, 1, 2, 10, OPMVX, 1)).starts_with("vmul.vx"));
    assert!(disassemble(vec_arith(0b100001, 1, 2, 10, OPMVX, 1)).starts_with("vdiv.vx"));
}

// ══════════════════════════════════════════════════════════
// Comparison
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_comparison() {
    assert!(disassemble(vec_arith(0b011000, 1, 2, 3, OPIVV, 1)).starts_with("vmseq.vv"));
    assert!(disassemble(vec_arith(0b011001, 1, 2, 3, OPIVV, 1)).starts_with("vmsne.vv"));
    assert!(disassemble(vec_arith(0b011010, 1, 2, 3, OPIVV, 1)).starts_with("vmsltu.vv"));
    assert!(disassemble(vec_arith(0b011011, 1, 2, 3, OPIVV, 1)).starts_with("vmslt.vv"));
    assert!(disassemble(vec_arith(0b011100, 1, 2, 3, OPIVV, 1)).starts_with("vmsleu.vv"));
    assert!(disassemble(vec_arith(0b011101, 1, 2, 3, OPIVV, 1)).starts_with("vmsle.vv"));
    assert!(disassemble(vec_arith(0b011110, 1, 2, 3, OPIVX, 1)).starts_with("vmsgtu.vx"));
    assert!(disassemble(vec_arith(0b011111, 1, 2, 3, OPIVX, 1)).starts_with("vmsgt.vx"));
}

// ══════════════════════════════════════════════════════════
// Widening / narrowing
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_widening() {
    assert!(disassemble(vec_arith(0b110000, 1, 2, 3, OPMVV, 1)).starts_with("vwaddu.vv"));
    assert!(disassemble(vec_arith(0b110001, 1, 2, 3, OPMVV, 1)).starts_with("vwadd.vv"));
    assert!(disassemble(vec_arith(0b111000, 1, 2, 3, OPMVV, 1)).starts_with("vwmulu.vv"));
    assert!(disassemble(vec_arith(0b111011, 1, 2, 3, OPMVV, 1)).starts_with("vwmul.vv"));

    // Narrowing
    assert!(disassemble(vec_arith(0b101100, 1, 2, 3, OPIVV, 1)).starts_with("vnsrl.vv"));
    assert!(disassemble(vec_arith(0b101101, 1, 2, 3, OPIVV, 1)).starts_with("vnsra.vv"));
}

// ══════════════════════════════════════════════════════════
// FP arithmetic
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_fp_arith() {
    assert!(disassemble(vec_arith(0b000000, 1, 2, 3, OPFVV, 1)).starts_with("vfadd.vv"));
    assert!(disassemble(vec_arith(0b000010, 1, 2, 3, OPFVV, 1)).starts_with("vfsub.vv"));
    assert!(disassemble(vec_arith(0b100100, 1, 2, 3, OPFVV, 1)).starts_with("vfmul.vv"));
    assert!(disassemble(vec_arith(0b100000, 1, 2, 3, OPFVV, 1)).starts_with("vfdiv.vv"));

    // vfsqrt.v — VFUNARY1, vs1=0b00000
    let inst = vec_arith(0b010011, 1, 2, 0b00000, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfsqrt.v"), "{}", disassemble(inst));

    // vfrsqrt7.v
    let inst = vec_arith(0b010011, 1, 2, 0b00100, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfrsqrt7.v"), "{}", disassemble(inst));

    // vfrec7.v
    let inst = vec_arith(0b010011, 1, 2, 0b00101, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfrec7.v"), "{}", disassemble(inst));

    // vfclass.v
    let inst = vec_arith(0b010011, 1, 2, 0b10000, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfclass.v"), "{}", disassemble(inst));

    // FMA
    assert!(disassemble(vec_arith(0b101101, 1, 2, 3, OPFVV, 1)).starts_with("vfmacc.vv"));
    assert!(disassemble(vec_arith(0b101001, 1, 2, 3, OPFVV, 1)).starts_with("vfmadd.vv"));

    // .vf variants
    assert!(disassemble(vec_arith(0b000000, 1, 2, 3, OPFVF, 1)).starts_with("vfadd.vf"));
    assert!(disassemble(vec_arith(0b100100, 1, 2, 3, OPFVF, 1)).starts_with("vfmul.vf"));
    assert!(disassemble(vec_arith(0b100001, 1, 2, 3, OPFVF, 1)).starts_with("vfrdiv.vf"));

    // Conversion: vfcvt.x.f.v — VFUNARY0, vs1=0b00001
    let inst = vec_arith(0b010010, 1, 2, 0b00001, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfcvt.x.f.v"), "{}", disassemble(inst));

    // vfwcvt.f.f.v — VFUNARY0, vs1=0b01100
    let inst = vec_arith(0b010010, 1, 2, 0b01100, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfwcvt.f.f.v"), "{}", disassemble(inst));

    // vfncvt.f.f.w — VFUNARY0, vs1=0b10100
    let inst = vec_arith(0b010010, 1, 2, 0b10100, OPFVV, 1);
    assert!(disassemble(inst).starts_with("vfncvt.f.f.w"), "{}", disassemble(inst));
}

// ══════════════════════════════════════════════════════════
// FP comparison
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_fp_compare() {
    assert!(disassemble(vec_arith(0b011000, 1, 2, 3, OPFVV, 1)).starts_with("vmfeq.vv"));
    assert!(disassemble(vec_arith(0b011011, 1, 2, 3, OPFVV, 1)).starts_with("vmflt.vv"));
    assert!(disassemble(vec_arith(0b011001, 1, 2, 3, OPFVV, 1)).starts_with("vmfle.vv"));
    assert!(disassemble(vec_arith(0b011100, 1, 2, 3, OPFVV, 1)).starts_with("vmfne.vv"));

    // vmfgt.vf / vmfge.vf — only available as .vf
    assert!(disassemble(vec_arith(0b011101, 1, 2, 3, OPFVF, 1)).starts_with("vmfgt.vf"));
    assert!(disassemble(vec_arith(0b011110, 1, 2, 3, OPFVF, 1)).starts_with("vmfge.vf"));
}

// ══════════════════════════════════════════════════════════
// Loads and stores
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_loads_stores() {
    // vle32.v v1, (a0)  — width=0b110 (e32), mop=00, lumop=00000, vm=1
    let inst = vec_load_unit(0, 0, 0b00, 1, 0b00000, 10, 0b110, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vle32.v"), "got '{text}'");
    assert!(text.contains("a0"), "expected a0 in '{text}'");

    // vlse64.v v2, (a1), a2 — width=0b111 (e64), mop=10 (strided)
    let inst = vec_load_rs2(0, 0, 0b10, 1, 12, 11, 0b111, 2);
    let text = disassemble(inst);
    assert!(text.starts_with("vlse64.v"), "got '{text}'");

    // vluxei32.v v3, (a0), v4 — width=0b110, mop=01 (indexed unordered)
    let inst = vec_load_rs2(0, 0, 0b01, 1, 4, 10, 0b110, 3);
    let text = disassemble(inst);
    assert!(text.starts_with("vluxei32.v"), "got '{text}'");

    // vloxei32.v v3, (a0), v4 — mop=11 (indexed ordered)
    let inst = vec_load_rs2(0, 0, 0b11, 1, 4, 10, 0b110, 3);
    let text = disassemble(inst);
    assert!(text.starts_with("vloxei32.v"), "got '{text}'");

    // vlm.v v0, (a0) — width=0b000 (e8), mop=00, lumop=01011
    let inst = vec_load_unit(0, 0, 0b00, 1, 0b01011, 10, 0b000, 0);
    let text = disassemble(inst);
    assert!(text.starts_with("vlm.v"), "got '{text}'");

    // vl1re8.v v1, (a0) — nf=0, lumop=01000, width=0b000 (e8)
    let inst = vec_load_unit(0, 0, 0b00, 1, 0b01000, 10, 0b000, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vl1re8.v"), "got '{text}'");

    // vle32ff.v v1, (a0) — fault-only-first, lumop=10000
    let inst = vec_load_unit(0, 0, 0b00, 1, 0b10000, 10, 0b110, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vle32ff.v"), "got '{text}'");

    // vse32.v v1, (a0)
    let inst = vec_store_unit(0, 0, 0b00, 1, 0b00000, 10, 0b110, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vse32.v"), "got '{text}'");

    // vsm.v v0, (a0)
    let inst = vec_store_unit(0, 0, 0b00, 1, 0b01011, 10, 0b000, 0);
    let text = disassemble(inst);
    assert!(text.starts_with("vsm.v"), "got '{text}'");

    // vsse32.v v1, (a0), a2 — strided store
    let inst = vec_store_rs2(0, 0, 0b10, 1, 12, 10, 0b110, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vsse32.v"), "got '{text}'");

    // Segment load: vlseg2e32.v — nf=1
    let inst = vec_load_unit(1, 0, 0b00, 1, 0b00000, 10, 0b110, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vlseg2e32.v"), "got '{text}'");
}

// ══════════════════════════════════════════════════════════
// Mask operations
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_mask() {
    // vmand.mm v1, v2, v3  (funct6=0b011001 under OPMVV)
    let inst = vec_arith(0b011001, 1, 2, 3, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vmand.mm"), "{}", disassemble(inst));

    // vmnand.mm
    let inst = vec_arith(0b011101, 1, 2, 3, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vmnand.mm"), "{}", disassemble(inst));

    // vmor.mm
    let inst = vec_arith(0b011010, 1, 2, 3, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vmor.mm"), "{}", disassemble(inst));

    // vmxor.mm
    let inst = vec_arith(0b011011, 1, 2, 3, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vmxor.mm"), "{}", disassemble(inst));

    // vcpop.m a0, v2  (funct6=0b010000, vs1=0b10000, OPMVV)
    let inst = vec_arith(0b010000, 1, 2, 0b10000, OPMVV, 10);
    let text = disassemble(inst);
    assert!(text.starts_with("vcpop.m"), "got '{text}'");
    assert!(text.contains("a0"), "expected a0 in '{text}'");

    // vfirst.m a0, v2  (funct6=0b010000, vs1=0b10001, OPMVV)
    let inst = vec_arith(0b010000, 1, 2, 0b10001, OPMVV, 10);
    let text = disassemble(inst);
    assert!(text.starts_with("vfirst.m"), "got '{text}'");

    // viota.m v1, v2  (funct6=0b010100, vs1=0b10000, OPMVV)
    let inst = vec_arith(0b010100, 1, 2, 0b10000, OPMVV, 1);
    assert!(disassemble(inst).starts_with("viota.m"), "{}", disassemble(inst));

    // vid.v v1  (funct6=0b010100, vs1=0b10001, OPMVV)
    let inst = vec_arith(0b010100, 1, 0, 0b10001, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vid.v"), "{}", disassemble(inst));

    // vzext.vf2 v1, v2  (funct6=0b010010, vs1=0b00110, OPMVV)
    let inst = vec_arith(0b010010, 1, 2, 0b00110, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vzext.vf2"), "{}", disassemble(inst));

    // vsext.vf4 v1, v2
    let inst = vec_arith(0b010010, 1, 2, 0b00101, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vsext.vf4"), "{}", disassemble(inst));
}

// ══════════════════════════════════════════════════════════
// Permute operations
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_permute() {
    // vslideup.vi — funct6=0b001110, OPIVI
    let inst = vec_arith(0b001110, 1, 2, 4, OPIVI, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vslideup.vi"), "got '{text}'");

    // vslideup.vx — OPIVX, funct6=0b001110
    let inst = vec_arith(0b001110, 1, 2, 10, OPIVX, 1);
    assert!(disassemble(inst).starts_with("vslideup.vx"), "{}", disassemble(inst));

    // vrgatherei16.vv — same funct6=0b001110 but under OPIVV
    let inst = vec_arith(0b001110, 1, 2, 3, OPIVV, 1);
    assert!(disassemble(inst).starts_with("vrgatherei16.vv"), "{}", disassemble(inst));

    // vrgather.vv
    let inst = vec_arith(0b001100, 1, 2, 3, OPIVV, 1);
    assert!(disassemble(inst).starts_with("vrgather.vv"), "{}", disassemble(inst));

    // vslidedown.vx — OPIVX, funct6=0b001111
    let inst = vec_arith(0b001111, 1, 2, 10, OPIVX, 1);
    assert!(disassemble(inst).starts_with("vslidedown.vx"), "{}", disassemble(inst));

    // vslideup.vx — OPMVX, funct6=0b001110
    let inst = vec_arith(0b001110, 1, 2, 10, OPMVX, 1);
    assert!(disassemble(inst).starts_with("vslide1up.vx"), "{}", disassemble(inst));

    // vslide1down.vx
    let inst = vec_arith(0b001111, 1, 2, 10, OPMVX, 1);
    assert!(disassemble(inst).starts_with("vslide1down.vx"), "{}", disassemble(inst));

    // vcompress.vm v1, v2, v3  (funct6=0b010111, OPMVV)
    let inst = vec_arith(0b010111, 1, 2, 3, OPMVV, 1);
    assert!(disassemble(inst).starts_with("vcompress.vm"), "{}", disassemble(inst));

    // vmv1r.v v1, v2  (funct6=0b100111, vs1=0 (nr-1=0), OPIVV)
    let inst = vec_arith(0b100111, 1, 2, 0, OPIVV, 1);
    assert!(disassemble(inst).starts_with("vmv1r.v"), "{}", disassemble(inst));

    // vmv4r.v v4, v8  (vs1=3)
    let inst = vec_arith(0b100111, 1, 8, 3, OPIVV, 4);
    assert!(disassemble(inst).starts_with("vmv4r.v"), "{}", disassemble(inst));

    // vmv.x.s a0, v2  (funct6=0b010000, vs1=0, OPMVV)
    let inst = vec_arith(0b010000, 1, 2, 0, OPMVV, 10);
    let text = disassemble(inst);
    assert!(text.starts_with("vmv.x.s"), "got '{text}'");
    assert!(text.contains("a0"), "expected a0 in '{text}'");

    // vmv.s.x v1, a0  (funct6=0b010000, OPMVX)
    let inst = vec_arith(0b010000, 1, 0, 10, OPMVX, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vmv.s.x"), "got '{text}'");

    // vfmv.f.s fa0, v2  (funct6=0b010000, vs1=0, OPFVV)
    let inst = vec_arith(0b010000, 1, 2, 0, OPFVV, 10);
    let text = disassemble(inst);
    assert!(text.starts_with("vfmv.f.s"), "got '{text}'");
    assert!(text.contains("fa0"), "expected fa0 in '{text}'");

    // vfmv.s.f v1, fa0  (funct6=0b010000, OPFVF)
    let inst = vec_arith(0b010000, 1, 0, 10, OPFVF, 1);
    let text = disassemble(inst);
    assert!(text.starts_with("vfmv.s.f"), "got '{text}'");
}

// ══════════════════════════════════════════════════════════
// Widening reductions
// ══════════════════════════════════════════════════════════

#[test]
fn test_rvv_reductions() {
    assert!(disassemble(vec_arith(0b000000, 1, 2, 3, OPMVV, 1)).starts_with("vredsum.vs"));
    assert!(disassemble(vec_arith(0b000001, 1, 2, 3, OPMVV, 1)).starts_with("vredand.vs"));
    assert!(disassemble(vec_arith(0b000010, 1, 2, 3, OPMVV, 1)).starts_with("vredor.vs"));
    assert!(disassemble(vec_arith(0b000011, 1, 2, 3, OPMVV, 1)).starts_with("vredxor.vs"));

    // FP reductions
    assert!(disassemble(vec_arith(0b000001, 1, 2, 3, OPFVV, 1)).starts_with("vfredusum.vs"));
    assert!(disassemble(vec_arith(0b000011, 1, 2, 3, OPFVV, 1)).starts_with("vfredosum.vs"));
}
