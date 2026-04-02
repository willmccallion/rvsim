//! Vector instruction disassembler for RVV 1.0.
//!
//! Provides human-readable disassembly for all vector arithmetic, load, and
//! store instructions encoded in the OP-V, OP-LOAD-FP, and OP-STORE-FP
//! major opcodes.

use crate::isa::rvv::{encoding, funct3 as vf3, funct6 as f6};

/// ABI-style vector register names v0–v31.
const VREG_NAMES: [&str; 32] = [
    "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", "v9", "v10", "v11", "v12", "v13", "v14",
    "v15", "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25", "v26", "v27",
    "v28", "v29", "v30", "v31",
];

/// ABI integer register names (duplicated here to avoid circular dep on disasm).
const XREG_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

const FREG_NAMES: [&str; 32] = [
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

#[inline]
fn vreg(idx: u8) -> &'static str {
    VREG_NAMES.get(idx as usize).copied().unwrap_or("v??")
}

#[inline]
fn xreg(idx: u8) -> &'static str {
    XREG_NAMES.get(idx as usize).copied().unwrap_or("x??")
}

#[inline]
fn freg(idx: u8) -> &'static str {
    FREG_NAMES.get(idx as usize).copied().unwrap_or("f??")
}

/// Returns `", v0.t"` for masked instructions (vm=0), `""` for unmasked (vm=1).
#[inline]
const fn vm_suffix(inst: u32) -> &'static str {
    if encoding::vm(inst) { "" } else { ", v0.t" }
}

/// Returns the element width string from the width/funct3 field.
const fn eew_str(width: u32) -> &'static str {
    match width {
        0b000 => "8",
        0b101 => "16",
        0b110 => "32",
        0b111 => "64",
        _ => "??",
    }
}

/// Formats a vtype immediate into a human-readable string like `"e32, m4, ta, ma"`.
fn vtype_str(zimm: u64) -> String {
    let sew = match (zimm >> 3) & 0x7 {
        0 => "e8",
        1 => "e16",
        2 => "e32",
        3 => "e64",
        _ => "e??",
    };
    let lmul = match zimm & 0x7 {
        0 => "m1",
        1 => "m2",
        2 => "m4",
        3 => "m8",
        5 => "mf8",
        6 => "mf4",
        7 => "mf2",
        _ => "m??",
    };
    let ta = if (zimm >> 6) & 1 != 0 { "ta" } else { "tu" };
    let ma = if (zimm >> 7) & 1 != 0 { "ma" } else { "mu" };
    format!("{sew}, {lmul}, {ta}, {ma}")
}

// ── OP-V arithmetic ─────────────────────────────────────────────────────────

/// Disassemble a vector arithmetic instruction (OP-V, opcode 0x57).
pub(crate) fn disasm_vec_arith(inst: u32) -> String {
    let f3 = (inst >> 12) & 0x7;
    let f6val = encoding::funct6(inst);
    let vd = encoding::vd(inst);
    let vs1 = encoding::vs1(inst);
    let vs2 = encoding::vs2(inst);
    let vm = vm_suffix(inst);

    match f3 {
        vf3::OPCFG => disasm_cfg(inst),

        vf3::OPIVV => {
            if f6val == 0b100111 {
                // vmv<nr>r.v — whole register move
                let nr = vs1 as u32 + 1;
                return format!("vmv{nr}r.v {}, {}", vreg(vd), vreg(vs2));
            }
            // funct6=0b001110 is vrgatherei16 under OPIVV (not vslideup)
            let mn = if f6val == 0b001110 { "vrgatherei16" } else { ivv_mnemonic(f6val) };
            format!("{mn}.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        vf3::OPIVX => {
            let mn = ivx_mnemonic(f6val);
            format!("{mn}.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(vs1))
        }
        vf3::OPIVI => {
            let mn = ivx_mnemonic(f6val);
            let simm = encoding::simm5(inst);
            format!("{mn}.vi {}, {}, {simm}{vm}", vreg(vd), vreg(vs2))
        }

        vf3::OPMVV => disasm_opmvv(inst, f6val, vd, vs1, vs2, vm),
        vf3::OPMVX => disasm_opmvx(inst, f6val, vd, vs1, vs2, vm),

        vf3::OPFVV => disasm_opfvv(inst, f6val, vd, vs1, vs2, vm),
        vf3::OPFVF => disasm_opfvf(inst, f6val, vd, vs1, vs2, vm),

        _ => format!("vec?? ({inst:#010x})"),
    }
}

/// Disassemble vsetvli / vsetivli / vsetvl.
fn disasm_cfg(inst: u32) -> String {
    let rd = encoding::vd(inst); // rd is in bits 11:7
    let rs1 = encoding::vs1(inst);
    let bit31 = (inst >> 31) & 1;
    let bit30 = (inst >> 30) & 1;

    if bit31 == 0 {
        // vsetvli rd, rs1, zimm[10:0]
        let zimm = encoding::zimm_vsetvli(inst);
        let vt = vtype_str(zimm);
        format!("vsetvli {}, {}, {vt}", xreg(rd), xreg(rs1))
    } else if bit30 == 1 {
        // vsetivli rd, uimm, zimm[9:0]
        let uimm = encoding::uimm_vsetivli(inst);
        let zimm = encoding::zimm_vsetivli(inst);
        let vt = vtype_str(zimm);
        format!("vsetivli {}, {uimm}, {vt}", xreg(rd))
    } else {
        // vsetvl rd, rs1, rs2
        let rs2 = encoding::vs2(inst);
        format!("vsetvl {}, {}, {}", xreg(rd), xreg(rs1), xreg(rs2))
    }
}

/// Integer VV mnemonic from funct6 (OPIVV only — no vslideup overlap).
const fn ivv_mnemonic(f6val: u32) -> &'static str {
    match f6val {
        f6::VADD => "vadd",
        f6::VSUB => "vsub",
        f6::VRSUB => "vrsub",
        f6::VMINU => "vminu",
        f6::VMIN => "vmin",
        f6::VMAXU => "vmaxu",
        f6::VMAX => "vmax",
        f6::VAND => "vand",
        f6::VOR => "vor",
        f6::VXOR => "vxor",
        f6::VRGATHER => "vrgather",
        f6::VSLIDEDOWN => "vslidedown",
        f6::VADC => "vadc",
        f6::VMADC => "vmadc",
        f6::VSBC => "vsbc",
        f6::VMSBC => "vmsbc",
        f6::VMERGE_VMV => "vmerge",
        f6::VMSEQ => "vmseq",
        f6::VMSNE => "vmsne",
        f6::VMSLTU => "vmsltu",
        f6::VMSLT => "vmslt",
        f6::VMSLEU => "vmsleu",
        f6::VMSLE => "vmsle",
        f6::VMSGTU => "vmsgtu",
        f6::VMSGT => "vmsgt",
        f6::VSADDU => "vsaddu",
        f6::VSADD => "vsadd",
        f6::VSSUBU => "vssubu",
        f6::VSSUB => "vssub",
        f6::VSLL => "vsll",
        f6::VSMUL => "vsmul",
        f6::VSRL => "vsrl",
        f6::VSRA => "vsra",
        f6::VSSRL => "vssrl",
        f6::VSSRA => "vssra",
        f6::VNSRL => "vnsrl",
        f6::VNSRA => "vnsra",
        f6::VNCLIPU => "vnclipu",
        f6::VNCLIP => "vnclip",
        _ => "vi??",
    }
}

/// Integer VX/VI mnemonic — handles vslideup (funct6=0b001110 is vslideup, not vrgatherei16).
const fn ivx_mnemonic(f6val: u32) -> &'static str {
    match f6val {
        0b001110 => "vslideup",
        _ => ivv_mnemonic(f6val),
    }
}

/// Disassemble OPMVV (funct3=010) — reductions, mul/div, mask, int extension.
fn disasm_opmvv(inst: u32, f6val: u32, vd: u8, vs1: u8, vs2: u8, vm: &str) -> String {
    match f6val {
        // Reductions
        f6::VREDSUM => format!("vredsum.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDAND => format!("vredand.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDOR => format!("vredor.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDXOR => format!("vredxor.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDMINU => format!("vredminu.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDMIN => format!("vredmin.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDMAXU => format!("vredmaxu.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREDMAX => format!("vredmax.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // Averaging
        f6::VAADDU => format!("vaaddu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VAADD => format!("vaadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VASUBU => format!("vasubu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VASUB => format!("vasub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // Mask logical (funct6 values overlap with comparison in OPIVV context)
        0b011000 => format!("vmandn.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011001 => format!("vmand.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011010 => format!("vmor.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011011 => format!("vmxor.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011100 => format!("vmorn.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011101 => format!("vmnand.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011110 => format!("vmnor.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        0b011111 => format!("vmxnor.mm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        // VRXUNARY0/VWXUNARY0 — vmv.x.s, vcpop.m, vfirst.m
        0b010000 => match vs1 {
            0b00000 => format!("vmv.x.s {}, {}", xreg(vd), vreg(vs2)),
            0b10000 => format!("vcpop.m {}, {}{vm}", xreg(vd), vreg(vs2)),
            0b10001 => format!("vfirst.m {}, {}{vm}", xreg(vd), vreg(vs2)),
            _ => format!("vwxunary0?? ({inst:#010x})"),
        },
        // VXUNARY0 — vzext, vsext
        0b010010 => {
            let mn = match vs1 {
                0b00010 => "vzext.vf8",
                0b00011 => "vsext.vf8",
                0b00100 => "vzext.vf4",
                0b00101 => "vsext.vf4",
                0b00110 => "vzext.vf2",
                0b00111 => "vsext.vf2",
                _ => return format!("vxunary0?? ({inst:#010x})"),
            };
            format!("{mn} {}, {}{vm}", vreg(vd), vreg(vs2))
        }
        // VMUNARY0 — vmsbf, vmsof, vmsif, viota, vid
        0b010100 => {
            let mn = match vs1 {
                0b00001 => "vmsbf.m",
                0b00010 => "vmsof.m",
                0b00011 => "vmsif.m",
                0b10000 => "viota.m",
                0b10001 => return format!("vid.v {}{vm}", vreg(vd)),
                _ => return format!("vmunary0?? ({inst:#010x})"),
            };
            format!("{mn} {}, {}{vm}", vreg(vd), vreg(vs2))
        }
        // vcompress
        0b010111 => format!("vcompress.vm {}, {}, {}", vreg(vd), vreg(vs2), vreg(vs1)),
        // Mul/div
        f6::VDIVU => format!("vdivu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VDIV => format!("vdiv.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREMU => format!("vremu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VREM => format!("vrem.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMULHU => format!("vmulhu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMUL => format!("vmul.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMULHSU => format!("vmulhsu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMULH => format!("vmulh.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMADD => format!("vmadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VNMSUB => format!("vnmsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VMACC => format!("vmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VNMSAC => format!("vnmsac.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        // Widening integer
        f6::VWADDU => format!("vwaddu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWADD => format!("vwadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWSUBU => format!("vwsubu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWSUB => format!("vwsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWADDU_W => format!("vwaddu.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWADD_W => format!("vwadd.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWSUBU_W => format!("vwsubu.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWSUB_W => format!("vwsub.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // Widening multiply
        f6::VWMULU => format!("vwmulu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWMULSU => format!("vwmulsu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWMUL => format!("vwmul.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VWMACCU => format!("vwmaccu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VWMACC => format!("vwmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VWMACCSU => format!("vwmaccsu.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        _ => {
            let _ = inst;
            format!("opmvv?? ({inst:#010x})")
        }
    }
}

/// Disassemble OPMVX (funct3=110).
fn disasm_opmvx(inst: u32, f6val: u32, vd: u8, rs1: u8, vs2: u8, vm: &str) -> String {
    match f6val {
        // vmv.s.x
        0b010000 => format!("vmv.s.x {}, {}", vreg(vd), xreg(rs1)),
        // vslide1up/down
        0b001110 => format!("vslide1up.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        0b001111 => format!("vslide1down.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        // Averaging
        f6::VAADDU => format!("vaaddu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VAADD => format!("vaadd.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VASUBU => format!("vasubu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VASUB => format!("vasub.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        // Mul/div
        f6::VDIVU => format!("vdivu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VDIV => format!("vdiv.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VREMU => format!("vremu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VREM => format!("vrem.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VMULHU => format!("vmulhu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VMUL => format!("vmul.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VMULHSU => format!("vmulhsu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VMULH => format!("vmulh.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VMADD => format!("vmadd.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VNMSUB => format!("vnmsub.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VMACC => format!("vmacc.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VNMSAC => format!("vnmsac.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        // Widening integer
        f6::VWADDU => format!("vwaddu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWADD => format!("vwadd.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWSUBU => format!("vwsubu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWSUB => format!("vwsub.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWADDU_W => format!("vwaddu.wx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWADD_W => format!("vwadd.wx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWSUBU_W => format!("vwsubu.wx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWSUB_W => format!("vwsub.wx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        // Widening multiply
        f6::VWMULU => format!("vwmulu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWMULSU => format!("vwmulsu.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWMUL => format!("vwmul.vx {}, {}, {}{vm}", vreg(vd), vreg(vs2), xreg(rs1)),
        f6::VWMACCU => format!("vwmaccu.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VWMACC => format!("vwmacc.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VWMACCUS => format!("vwmaccus.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        f6::VWMACCSU => format!("vwmaccsu.vx {}, {}, {}{vm}", vreg(vd), xreg(rs1), vreg(vs2)),
        _ => {
            let _ = inst;
            format!("opmvx?? ({inst:#010x})")
        }
    }
}

/// Disassemble OPFVV (funct3=001).
fn disasm_opfvv(inst: u32, f6val: u32, vd: u8, vs1: u8, vs2: u8, vm: &str) -> String {
    match f6val {
        f6::VFADD => format!("vfadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFREDUSUM => {
            format!("vfredusum.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFSUB => format!("vfsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFREDOSUM => {
            format!("vfredosum.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFMIN => format!("vfmin.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFREDMIN => {
            format!("vfredmin.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFMAX => format!("vfmax.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFREDMAX => {
            format!("vfredmax.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFSGNJ => format!("vfsgnj.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFSGNJN => format!("vfsgnjn.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFSGNJX => format!("vfsgnjx.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // VFUNARY0 — conversion unaries, sub-dispatch on vs1
        f6::VFUNARY0 => disasm_vfunary0(inst, vd, vs1, vs2, vm),
        // VFUNARY1 — sqrt, rsqrt, rec, class
        f6::VFUNARY1 => disasm_vfunary1(inst, vd, vs1, vs2, vm),
        // VWFUNARY0 — vfmv.f.s
        0b010000 => format!("vfmv.f.s {}, {}", freg(vd), vreg(vs2)),
        // Comparisons
        f6::VMFEQ => format!("vmfeq.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMFLE => format!("vmfle.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMFORD => format!("vmford.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMFLT => format!("vmflt.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VMFNE => format!("vmfne.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // FP arithmetic
        f6::VFDIV => format!("vfdiv.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFMUL => format!("vfmul.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        // FMA
        f6::VFMADD => format!("vfmadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFNMADD => format!("vfnmadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFMSUB => format!("vfmsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFNMSUB => format!("vfnmsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFMACC => format!("vfmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFNMACC => format!("vfnmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFMSAC => format!("vfmsac.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFNMSAC => format!("vfnmsac.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        // Widening FP
        f6::VFWADD => format!("vfwadd.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFWREDUSUM => {
            format!("vfwredusum.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFWSUB => format!("vfwsub.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFWREDOSUM => {
            format!("vfwredosum.vs {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1))
        }
        f6::VFWADD_W => format!("vfwadd.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFWSUB_W => format!("vfwsub.wv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFWMUL => format!("vfwmul.vv {}, {}, {}{vm}", vreg(vd), vreg(vs2), vreg(vs1)),
        f6::VFWMACC => format!("vfwmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFWNMACC => {
            format!("vfwnmacc.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2))
        }
        f6::VFWMSAC => format!("vfwmsac.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2)),
        f6::VFWNMSAC => {
            format!("vfwnmsac.vv {}, {}, {}{vm}", vreg(vd), vreg(vs1), vreg(vs2))
        }
        _ => {
            let _ = inst;
            format!("opfvv?? ({inst:#010x})")
        }
    }
}

/// Disassemble VFUNARY0 (funct6=010010, OPFVV) — conversion unaries.
fn disasm_vfunary0(_inst: u32, vd: u8, vs1: u8, vs2: u8, vm: &str) -> String {
    let mn = match vs1 {
        0b00000 => "vfcvt.xu.f.v",
        0b00001 => "vfcvt.x.f.v",
        0b00010 => "vfcvt.f.xu.v",
        0b00011 => "vfcvt.f.x.v",
        0b00110 => "vfcvt.rtz.xu.f.v",
        0b00111 => "vfcvt.rtz.x.f.v",
        0b01000 => "vfwcvt.xu.f.v",
        0b01001 => "vfwcvt.x.f.v",
        0b01010 => "vfwcvt.f.xu.v",
        0b01011 => "vfwcvt.f.x.v",
        0b01100 => "vfwcvt.f.f.v",
        0b01110 => "vfwcvt.rtz.xu.f.v",
        0b01111 => "vfwcvt.rtz.x.f.v",
        0b10000 => "vfncvt.xu.f.w",
        0b10001 => "vfncvt.x.f.w",
        0b10010 => "vfncvt.f.xu.w",
        0b10011 => "vfncvt.f.x.w",
        0b10100 => "vfncvt.f.f.w",
        0b10101 => "vfncvt.rod.f.f.w",
        0b10110 => "vfncvt.rtz.xu.f.w",
        0b10111 => "vfncvt.rtz.x.f.w",
        _ => return format!("vfunary0?? (vs1={vs1:#07b})"),
    };
    format!("{mn} {}, {}{vm}", vreg(vd), vreg(vs2))
}

/// Disassemble VFUNARY1 (funct6=010011, OPFVV) — sqrt, rsqrt7, rec7, class.
fn disasm_vfunary1(_inst: u32, vd: u8, vs1: u8, vs2: u8, vm: &str) -> String {
    let mn = match vs1 {
        0b00000 => "vfsqrt.v",
        0b00100 => "vfrsqrt7.v",
        0b00101 => "vfrec7.v",
        0b10000 => "vfclass.v",
        _ => return format!("vfunary1?? (vs1={vs1:#07b})"),
    };
    format!("{mn} {}, {}{vm}", vreg(vd), vreg(vs2))
}

/// Disassemble OPFVF (funct3=101).
fn disasm_opfvf(inst: u32, f6val: u32, vd: u8, rs1: u8, vs2: u8, vm: &str) -> String {
    match f6val {
        f6::VFADD => format!("vfadd.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFSUB => format!("vfsub.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFMIN => format!("vfmin.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFMAX => format!("vfmax.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFSGNJ => format!("vfsgnj.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFSGNJN => format!("vfsgnjn.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFSGNJX => format!("vfsgnjx.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFSLIDE1UP => {
            format!("vfslide1up.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1))
        }
        f6::VFSLIDE1DOWN => {
            format!("vfslide1down.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1))
        }
        // VRFUNARY0 — vfmv.s.f
        0b010000 => format!("vfmv.s.f {}, {}", vreg(vd), freg(rs1)),
        // Comparisons
        f6::VMFEQ => format!("vmfeq.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFLE => format!("vmfle.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFORD => format!("vmford.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFLT => format!("vmflt.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFNE => format!("vmfne.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFGT => format!("vmfgt.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VMFGE => format!("vmfge.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        // FP arithmetic
        f6::VFDIV => format!("vfdiv.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFRDIV => format!("vfrdiv.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFMUL => format!("vfmul.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        // FMA
        f6::VFMADD => format!("vfmadd.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFNMADD => format!("vfnmadd.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFMSUB => format!("vfmsub.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFNMSUB => format!("vfnmsub.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFMACC => format!("vfmacc.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFNMACC => format!("vfnmacc.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFMSAC => format!("vfmsac.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFNMSAC => format!("vfnmsac.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        // Widening FP
        f6::VFWADD => format!("vfwadd.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFWSUB => format!("vfwsub.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFWADD_W => format!("vfwadd.wf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFWSUB_W => format!("vfwsub.wf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFWMUL => format!("vfwmul.vf {}, {}, {}{vm}", vreg(vd), vreg(vs2), freg(rs1)),
        f6::VFWMACC => format!("vfwmacc.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFWNMACC => {
            format!("vfwnmacc.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2))
        }
        f6::VFWMSAC => format!("vfwmsac.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2)),
        f6::VFWNMSAC => {
            format!("vfwnmsac.vf {}, {}, {}{vm}", vreg(vd), freg(rs1), vreg(vs2))
        }
        _ => {
            let _ = inst;
            format!("opfvf?? ({inst:#010x})")
        }
    }
}

// ── Vector loads ────────────────────────────────────────────────────────────

/// Disassemble a vector load instruction (OP-LOAD-FP with vector funct3).
pub(crate) fn disasm_vec_load(inst: u32) -> String {
    let f3 = (inst >> 12) & 0x7;
    let eew = eew_str(f3);
    let vd = encoding::vd(inst);
    let rs1 = encoding::vs1(inst); // rs1 is in bits 19:15
    let mop_val = encoding::mop(inst);
    let nf_val = encoding::nf(inst);
    let vm = vm_suffix(inst);

    match mop_val {
        // Unit-stride
        0b00 => {
            let lumop_val = encoding::lumop(inst);
            match lumop_val {
                0b00000 => {
                    if nf_val > 0 {
                        let seg = nf_val + 1;
                        format!("vlseg{seg}e{eew}.v {}, ({}){vm}", vreg(vd), xreg(rs1))
                    } else {
                        format!("vle{eew}.v {}, ({}){vm}", vreg(vd), xreg(rs1))
                    }
                }
                0b01000 => {
                    // Whole-register load
                    let nregs = nf_val + 1;
                    format!("vl{nregs}re{eew}.v {}, ({})", vreg(vd), xreg(rs1))
                }
                0b01011 => {
                    // Mask load
                    format!("vlm.v {}, ({})", vreg(vd), xreg(rs1))
                }
                0b10000 => {
                    // Fault-only-first
                    if nf_val > 0 {
                        let seg = nf_val + 1;
                        format!("vlseg{seg}e{eew}ff.v {}, ({}){vm}", vreg(vd), xreg(rs1))
                    } else {
                        format!("vle{eew}ff.v {}, ({}){vm}", vreg(vd), xreg(rs1))
                    }
                }
                _ => format!("vl?? ({inst:#010x})"),
            }
        }
        // Strided
        0b10 => {
            let rs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vlsseg{seg}e{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), xreg(rs2))
            } else {
                format!("vlse{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), xreg(rs2))
            }
        }
        // Indexed unordered
        0b01 => {
            let vs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vluxseg{seg}ei{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), vreg(vs2))
            } else {
                format!("vluxei{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), vreg(vs2))
            }
        }
        // Indexed ordered
        0b11 => {
            let vs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vloxseg{seg}ei{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), vreg(vs2))
            } else {
                format!("vloxei{eew}.v {}, ({}), {}{vm}", vreg(vd), xreg(rs1), vreg(vs2))
            }
        }
        _ => format!("vl?? ({inst:#010x})"),
    }
}

// ── Vector stores ───────────────────────────────────────────────────────────

/// Disassemble a vector store instruction (OP-STORE-FP with vector funct3).
pub(crate) fn disasm_vec_store(inst: u32) -> String {
    let f3 = (inst >> 12) & 0x7;
    let eew = eew_str(f3);
    let vs3 = encoding::vd(inst); // store data register is in vd/rd position
    let rs1 = encoding::vs1(inst);
    let mop_val = encoding::mop(inst);
    let nf_val = encoding::nf(inst);
    let vm = vm_suffix(inst);

    match mop_val {
        // Unit-stride
        0b00 => {
            let sumop_val = encoding::sumop(inst);
            match sumop_val {
                0b00000 => {
                    if nf_val > 0 {
                        let seg = nf_val + 1;
                        format!("vsseg{seg}e{eew}.v {}, ({}){vm}", vreg(vs3), xreg(rs1))
                    } else {
                        format!("vse{eew}.v {}, ({}){vm}", vreg(vs3), xreg(rs1))
                    }
                }
                0b01000 => {
                    // Whole-register store
                    let nregs = nf_val + 1;
                    format!("vs{nregs}r.v {}, ({})", vreg(vs3), xreg(rs1))
                }
                0b01011 => {
                    // Mask store
                    format!("vsm.v {}, ({})", vreg(vs3), xreg(rs1))
                }
                _ => format!("vs?? ({inst:#010x})"),
            }
        }
        // Strided
        0b10 => {
            let rs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vssseg{seg}e{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), xreg(rs2))
            } else {
                format!("vsse{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), xreg(rs2))
            }
        }
        // Indexed unordered
        0b01 => {
            let vs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vsuxseg{seg}ei{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), vreg(vs2))
            } else {
                format!("vsuxei{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), vreg(vs2))
            }
        }
        // Indexed ordered
        0b11 => {
            let vs2 = encoding::vs2(inst);
            if nf_val > 0 {
                let seg = nf_val + 1;
                format!("vsoxseg{seg}ei{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), vreg(vs2))
            } else {
                format!("vsoxei{eew}.v {}, ({}), {}{vm}", vreg(vs3), xreg(rs1), vreg(vs2))
            }
        }
        _ => format!("vs?? ({inst:#010x})"),
    }
}
