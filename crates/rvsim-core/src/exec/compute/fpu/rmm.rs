//! Round-to-nearest-max-magnitude, which the host FPU lacks, emulated from
//! round-to-nearest-even results.

use crate::isa::op::AluOp;

use super::nan_handling::{box_f32_canon, canonicalize_f64_bits, unbox_f32};

/// Rounds an exact f64 value to f32 under RMM (round to nearest, ties to
/// max magnitude).
///
/// The host has no native RMM mode, so RMM arith is computed at RNE and
/// then fixed up on tie cases. For f32 ops where the exact result fits in
/// f64 (add/sub/mul of two f32 operands), we can compute exactly and apply
/// this rounding.
pub fn rmm_round_f64_to_f32(exact: f64) -> f32 {
    if !exact.is_finite() {
        return exact as f32;
    }
    let rne = exact as f32;
    let rne_d = rne as f64;
    if rne_d == exact {
        return rne;
    }
    // The neighbor of `rne` on the opposite side of `exact`.
    let rne_bits = rne.to_bits();
    let other_bits: u32 = if rne_d > exact {
        // rne is above exact → other is below rne.
        if rne == 0.0 {
            0x8000_0001
        } else if rne > 0.0 {
            rne_bits - 1
        } else {
            rne_bits + 1
        }
    } else {
        // rne is below exact → other is above rne.
        if rne == 0.0 {
            0x0000_0001
        } else if rne > 0.0 {
            rne_bits + 1
        } else {
            rne_bits - 1
        }
    };
    let other = f32::from_bits(other_bits);
    if !other.is_finite() {
        return rne;
    }
    // Midpoint is exact in f64 for two adjacent f32 values.
    let midpoint = (rne_d + other as f64) * 0.5;
    if exact == midpoint { if rne.abs() >= other.abs() { rne } else { other } } else { rne }
}

/// Fixes a host-RNE f64 add/sub result for RMM tie cases.
///
/// Uses Dekker's 2Sum to compute the exact rounding error `lo` in
/// `a + b_eff`. If `2*|lo|` equals the gap to the adjacent f64, the result
/// sat exactly at a half-ULP tie and RMM wants the max-magnitude neighbor.
pub(super) fn rmm_fix_f64_add_sub(a: f64, b_eff: f64) -> f64 {
    let hi = a + b_eff;
    if !hi.is_finite() || hi == 0.0 {
        return hi;
    }
    // Dekker 2Sum: hi + lo = a + b_eff exactly (no overflow).
    let bp = hi - a;
    let ap = hi - bp;
    let da = a - ap;
    let db = b_eff - bp;
    let lo = da + db;
    if lo == 0.0 {
        return hi;
    }
    // Adjacent f64 in the direction of `lo`'s sign (i.e. toward the exact value).
    let hi_bits = hi.to_bits();
    let adjacent_bits = if hi > 0.0 {
        if lo > 0.0 { hi_bits + 1 } else { hi_bits - 1 }
    } else if lo > 0.0 {
        hi_bits - 1
    } else {
        hi_bits + 1
    };
    let adjacent = f64::from_bits(adjacent_bits);
    if !adjacent.is_finite() {
        return hi;
    }
    let gap = (adjacent - hi).abs();
    if lo.abs() * 2.0 != gap {
        return hi; // not a half-ULP tie
    }
    if hi.abs() >= adjacent.abs() { hi } else { adjacent }
}

/// Dispatches to the correct RMM tie-fixup helper for an inexact
/// arithmetic result previously computed at host RNE.
///
/// Supported ops: f32 add/sub/mul (via f64 higher precision) and f64
/// add/sub (via Dekker 2Sum). Other ops fall through unchanged — their
/// RMM tie behavior remains an RNE approximation.
pub(super) fn rmm_fixup(op: AluOp, a: u64, b: u64, is32: bool, rne_result: u64) -> u64 {
    if is32 {
        let fa = unbox_f32(a) as f64;
        let fb = unbox_f32(b) as f64;
        let exact = match op {
            AluOp::FAdd => fa + fb,
            AluOp::FSub => fa - fb,
            AluOp::FMul => fa * fb,
            _ => return rne_result,
        };
        box_f32_canon(rmm_round_f64_to_f32(exact))
    } else {
        let fa = f64::from_bits(a);
        let fb = f64::from_bits(b);
        let fixed = match op {
            AluOp::FAdd => rmm_fix_f64_add_sub(fa, fb),
            AluOp::FSub => rmm_fix_f64_add_sub(fa, -fb),
            _ => return rne_result,
        };
        canonicalize_f64_bits(fixed)
    }
}
