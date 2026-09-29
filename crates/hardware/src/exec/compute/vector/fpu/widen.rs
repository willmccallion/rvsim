//! Widening and narrowing operations, including widening FMA.

use super::convert::{
    f32_to_i64_frm, f32_to_u64_frm, f64_to_f32_round_to_odd, f64_to_i32_frm, f64_to_u32_frm,
};
use super::{elem_to_f32, elem_to_f64, mask_active, read_op1, sign_extend, widen_sew};
use crate::exec::compute::fpu::half::{f16_to_f32, f64_to_f16};
use crate::exec::compute::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits};
use crate::exec::compute::fpu::{clear_host_fp_flags, read_host_fp_flags, rmm_round_f64_to_f32};
use crate::exec::compute::vector::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};

/// Widening FP operations: read at SEW, write at 2*SEW.
#[allow(clippy::too_many_lines)]
pub(super) fn exec_fp_widening(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };

    // Handle widening FMA BEFORE the main loop — the FMA reads vd as the
    // accumulator, so we must not overwrite it with the arithmetic path first.
    if matches!(op, VectorOp::VFWMacc | VectorOp::VFWNMacc | VectorOp::VFWMSac | VectorOp::VFWNMSac)
    {
        return exec_fp_widening_fma(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    // Determine if this is a ".w" variant (vs2 is already wide)
    let vs2_wide = matches!(op, VectorOp::VFWAddW | VectorOp::VFWSubW);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }

        // Read source elements
        let vs2_raw = if vs2_wide {
            vpr.read_element(vs2_idx, ElemIdx::new(i), wsew)
        } else {
            vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew)
        };
        let op1_raw = read_op1(vpr, &operand1, i, ctx.sew);

        // For widening FP: SEW=32 -> f32 inputs, f64 output
        if ctx.sew == Sew::E32 {
            // Handle integer-producing conversions (bypass the f64 path)
            if matches!(
                op,
                VectorOp::VFWCvtXuF
                    | VectorOp::VFWCvtXF
                    | VectorOp::VFWCvtRtzXuF
                    | VectorOp::VFWCvtRtzXF
                    | VectorOp::VFWCvtFXu
                    | VectorOp::VFWCvtFX
            ) {
                let (bits, f) = match op {
                    VectorOp::VFWCvtXuF => {
                        let a = elem_to_f32(vs2_raw);
                        let (r, fl) = f32_to_u64_frm(a, ctx.frm);
                        (r, fl)
                    }
                    VectorOp::VFWCvtXF => {
                        let a = elem_to_f32(vs2_raw);
                        let (r, fl) = f32_to_i64_frm(a, ctx.frm);
                        (r as u64, fl)
                    }
                    VectorOp::VFWCvtRtzXuF => {
                        let a = elem_to_f32(vs2_raw);
                        let (r, fl) = f32_to_u64_frm(a, RoundingMode::Rtz);
                        (r, fl)
                    }
                    VectorOp::VFWCvtRtzXF => {
                        let a = elem_to_f32(vs2_raw);
                        let (r, fl) = f32_to_i64_frm(a, RoundingMode::Rtz);
                        (r as u64, fl)
                    }
                    VectorOp::VFWCvtFXu => ((vs2_raw as u32 as f64).to_bits(), FpFlags::NONE),
                    VectorOp::VFWCvtFX => {
                        ((sign_extend(vs2_raw, ctx.sew) as i32 as f64).to_bits(), FpFlags::NONE)
                    }
                    _ => (0, FpFlags::NONE),
                };
                flags = flags | f;
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, bits);
                continue;
            }

            // FP arithmetic widening path
            let vs2_f = if vs2_wide { elem_to_f64(vs2_raw) } else { elem_to_f32(vs2_raw) as f64 };
            let op1_f = elem_to_f32(op1_raw) as f64;

            clear_host_fp_flags();
            let r = std::hint::black_box(match op {
                VectorOp::VFWAdd | VectorOp::VFWAddW => {
                    std::hint::black_box(vs2_f) + std::hint::black_box(op1_f)
                }
                VectorOp::VFWSub | VectorOp::VFWSubW => {
                    std::hint::black_box(vs2_f) - std::hint::black_box(op1_f)
                }
                VectorOp::VFWMul => std::hint::black_box(vs2_f) * std::hint::black_box(op1_f),
                _ => vs2_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, canonicalize_f64_bits(r));
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh widening: SEW=16 (f16) -> wsew=E32 (f32)
            // Handle integer-producing conversions
            if matches!(
                op,
                VectorOp::VFWCvtXuF
                    | VectorOp::VFWCvtXF
                    | VectorOp::VFWCvtRtzXuF
                    | VectorOp::VFWCvtRtzXF
                    | VectorOp::VFWCvtFXu
                    | VectorOp::VFWCvtFX
            ) {
                clear_host_fp_flags();
                let a16 = vs2_raw as u16;
                let a_f = f16_to_f32(a16);
                let bits = match op {
                    VectorOp::VFWCvtXuF | VectorOp::VFWCvtRtzXuF => {
                        if a_f.is_nan() {
                            u32::MAX as u64
                        } else {
                            a_f as u32 as u64
                        }
                    }
                    VectorOp::VFWCvtXF | VectorOp::VFWCvtRtzXF => {
                        if a_f.is_nan() {
                            i32::MAX as u64
                        } else {
                            a_f as i32 as u32 as u64
                        }
                    }
                    VectorOp::VFWCvtFXu => (vs2_raw as u16 as f32).to_bits() as u64,
                    VectorOp::VFWCvtFX => {
                        (sign_extend(vs2_raw, Sew::E16) as i16 as f32).to_bits() as u64
                    }
                    _ => 0,
                };
                let nan_input = matches!(
                    op,
                    VectorOp::VFWCvtXuF
                        | VectorOp::VFWCvtXF
                        | VectorOp::VFWCvtRtzXuF
                        | VectorOp::VFWCvtRtzXF
                ) && a_f.is_nan();
                let f = read_host_fp_flags() | if nan_input { FpFlags::NV } else { FpFlags::NONE };
                flags = flags | f;
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, bits);
                continue;
            }

            // FP arithmetic widening: f16 inputs → f32 output (via f64 for lossless computation)
            let vs2_f = if vs2_wide {
                elem_to_f32(vs2_raw) as f64
            } else {
                f16_to_f32(vs2_raw as u16) as f64
            };
            let op1_f = f16_to_f32(op1_raw as u16) as f64;

            clear_host_fp_flags();
            let r_f64 = std::hint::black_box(match op {
                VectorOp::VFWAdd | VectorOp::VFWAddW => {
                    std::hint::black_box(vs2_f) + std::hint::black_box(op1_f)
                }
                VectorOp::VFWSub | VectorOp::VFWSubW => {
                    std::hint::black_box(vs2_f) - std::hint::black_box(op1_f)
                }
                VectorOp::VFWMul => std::hint::black_box(vs2_f) * std::hint::black_box(op1_f),
                // VFWCvtFF (f16→f32) and other widening identity paths return
                // the input unchanged (the widen happens via the cast above).
                _ => vs2_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            // RMM has no native host equivalent (set_host_round_mode mapped it
            // to FE_TONEAREST). f16+f16 in f64 is exact, so the only rounding
            // happens in the f64→f32 cast at the end. Use rmm_round_f64_to_f32
            // to fix the half-ULP ties to max-magnitude under RMM.
            let r_f32 = if ctx.frm == RoundingMode::Rmm {
                rmm_round_f64_to_f32(r_f64)
            } else {
                r_f64 as f32
            };
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, box_f32_canon(r_f32));
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Widening FMA operations.
pub(super) fn exec_fp_widening_fma(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), wsew, wsew.ones());
            }
            continue;
        }

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_raw = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_raw = vpr.read_element(vd_idx, ElemIdx::new(i), wsew);

        if ctx.sew == Sew::E32 {
            let vs2_f = elem_to_f32(vs2_raw) as f64;
            let op1_f = elem_to_f32(op1_raw) as f64;
            let vd_f = elem_to_f64(vd_raw);

            clear_host_fp_flags();
            let r = std::hint::black_box(match op {
                VectorOp::VFWMacc => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                VectorOp::VFWNMacc => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWMSac => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWNMSac => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                _ => vd_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, canonicalize_f64_bits(r));
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh widening FMA: f16 inputs → f32 accumulator
            let vs2_f = f16_to_f32(vs2_raw as u16) as f64;
            let op1_f = f16_to_f32(op1_raw as u16) as f64;
            let vd_f = elem_to_f32(vd_raw) as f64;

            clear_host_fp_flags();
            let r = std::hint::black_box(match op {
                VectorOp::VFWMacc => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                VectorOp::VFWNMacc => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWMSac => std::hint::black_box(op1_f)
                    .mul_add(std::hint::black_box(vs2_f), -std::hint::black_box(vd_f)),
                VectorOp::VFWNMSac => (-std::hint::black_box(op1_f))
                    .mul_add(std::hint::black_box(vs2_f), std::hint::black_box(vd_f)),
                _ => vd_f,
            });
            let f = read_host_fp_flags();
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, box_f32_canon(r as f32));
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Narrowing FP operations: read at 2*SEW (or SEW for destination), write at SEW.
pub(super) fn exec_fp_narrowing(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    _operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // For narrowing, the destination SEW is ctx.sew, source is 2*ctx.sew
    let Some(src_sew) = widen_sew(ctx.sew) else {
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);

        // Narrowing: src_sew=E64 -> dst_sew=E32
        let (result, f) = if ctx.sew == Sew::E32 {
            let a64 = elem_to_f64(vs2_raw);
            match op {
                VectorOp::VFNCvtFF => {
                    clear_host_fp_flags();
                    let r = std::hint::black_box(std::hint::black_box(a64) as f32);
                    (box_f32_canon(r) & 0xFFFF_FFFF, read_host_fp_flags())
                }
                VectorOp::VFNCvtRodFF => {
                    // Round-to-odd: truncate to f32 precision and jam LSB to 1
                    // if any bit was lost. Round-to-odd does not raise the
                    // inexact flag (per Zvfh).
                    let r = f64_to_f32_round_to_odd(a64);
                    (box_f32_canon(r) & 0xFFFF_FFFF, FpFlags::NONE)
                }
                VectorOp::VFNCvtXuF => {
                    let (r, f) = f64_to_u32_frm(a64, ctx.frm);
                    (r as u64, f)
                }
                VectorOp::VFNCvtXF => {
                    let (r, f) = f64_to_i32_frm(a64, ctx.frm);
                    (r as u32 as u64, f)
                }
                VectorOp::VFNCvtRtzXuF => {
                    let (r, f) = f64_to_u32_frm(a64, RoundingMode::Rtz);
                    (r as u64, f)
                }
                VectorOp::VFNCvtRtzXF => {
                    let (r, f) = f64_to_i32_frm(a64, RoundingMode::Rtz);
                    (r as u32 as u64, f)
                }
                VectorOp::VFNCvtFXu => {
                    // Convert full 2*SEW unsigned integer to SEW float.
                    // Must not truncate to u32 first — the source is a 64-bit integer.
                    clear_host_fp_flags();
                    let r = std::hint::black_box(vs2_raw as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                VectorOp::VFNCvtFX => {
                    // Convert full 2*SEW signed integer to SEW float.
                    // Must not truncate to i32 first — the source is a 64-bit integer.
                    clear_host_fp_flags();
                    let r = std::hint::black_box(sign_extend(vs2_raw, src_sew) as f32);
                    (r.to_bits() as u64, read_host_fp_flags())
                }
                _ => (0, FpFlags::NONE),
            }
        } else if ctx.sew == Sew::E16 && ctx.zvfh {
            // Zvfh narrowing: src_sew=E32 (f32) -> dst_sew=E16 (f16)
            let a32 = elem_to_f32(vs2_raw);
            match op {
                VectorOp::VFNCvtFF => {
                    // f32 -> f16 with rounding mode
                    let (bits, f) = f64_to_f16(a32 as f64, ctx.frm);
                    (bits as u64, f)
                }
                VectorOp::VFNCvtRodFF => {
                    // Round-to-odd: round toward zero into f16, then jam the
                    // mantissa LSB to 1 if rounding was inexact (so the
                    // result is always odd when narrowing loses precision).
                    // Round-to-odd suppresses NX (per Zvfh).
                    let (bits, f) = f64_to_f16(a32 as f64, RoundingMode::Rtz);
                    let inexact = f.bits() & FpFlags::NX.bits() != 0;
                    let finite = !a32.is_nan() && !a32.is_infinite();
                    let jammed = if inexact && finite { bits | 1 } else { bits };
                    let flags_no_nx = FpFlags::from_bits(f.bits() & !FpFlags::NX.bits());
                    (jammed as u64, flags_no_nx)
                }
                VectorOp::VFNCvtXuF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() { u16::MAX as u64 } else { a32 as u16 as u64 };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtXF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() { i16::MAX as u64 } else { a32 as i16 as u16 as u64 };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtRtzXuF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() {
                        u16::MAX as u64
                    } else {
                        std::hint::black_box(a32) as u16 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtRtzXF => {
                    clear_host_fp_flags();
                    let r = if a32.is_nan() {
                        i16::MAX as u64
                    } else {
                        std::hint::black_box(a32) as i16 as u16 as u64
                    };
                    let f = read_host_fp_flags()
                        | if a32.is_nan() { FpFlags::NV } else { FpFlags::NONE };
                    (r, f)
                }
                VectorOp::VFNCvtFXu => {
                    // 2*SEW unsigned int (u32) → f16
                    let (bits, f) = f64_to_f16(vs2_raw as u32 as f64, ctx.frm);
                    (bits as u64, f)
                }
                VectorOp::VFNCvtFX => {
                    // 2*SEW signed int (i32) → f16
                    let (bits, f) =
                        f64_to_f16(sign_extend(vs2_raw, src_sew) as i32 as f64, ctx.frm);
                    (bits as u64, f)
                }
                _ => (0, FpFlags::NONE),
            }
        } else {
            (0u64, FpFlags::NONE)
        };

        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}
