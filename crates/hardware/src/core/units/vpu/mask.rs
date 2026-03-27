//! Vector mask operations.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) mask operations:
//! - Mask-register logical: `vmand`, `vmnand`, `vmandn`, `vmor`, `vmnor`,
//!   `vmorn`, `vmxor`, `vmxnor`
//! - Mask scalar: `vcpop.m`, `vfirst.m`
//! - Mask-producing: `vmsbf.m`, `vmsif.m`, `vmsof.m`
//! - Mask misc: `viota.m`, `vid.v`

use crate::core::pipeline::signals::VectorOp;
use crate::core::units::fpu::exception_flags::FpFlags;
use crate::core::units::vpu::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::core::units::vpu::regfile::VectorRegFile;
use crate::core::units::vpu::types::{ElemIdx, VRegIdx, Vlmax};

// ============================================================================
// Public API
// ============================================================================

/// Returns `true` if `op` is a mask operation handled by this module.
pub const fn is_mask_op(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMAndMM
            | VectorOp::VMNandMM
            | VectorOp::VMAndnMM
            | VectorOp::VMOrMM
            | VectorOp::VMNorMM
            | VectorOp::VMOrnMM
            | VectorOp::VMXorMM
            | VectorOp::VMXnorMM
            | VectorOp::VCPopM
            | VectorOp::VFirstM
            | VectorOp::VMSbfM
            | VectorOp::VMSifM
            | VectorOp::VMSofM
            | VectorOp::VIotaM
            | VectorOp::VIdV
    )
}

/// Execute a mask operation.
///
/// For scalar-producing ops ([`VectorOp::VCPopM`], [`VectorOp::VFirstM`]),
/// the result is returned in [`VecExecResult::scalar_result`]. For
/// vector-producing ops, results are written to `vd` in the VPR.
pub fn vec_mask_execute(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        // Mask-register logical
        VectorOp::VMAndMM
        | VectorOp::VMNandMM
        | VectorOp::VMAndnMM
        | VectorOp::VMOrMM
        | VectorOp::VMNorMM
        | VectorOp::VMOrnMM
        | VectorOp::VMXorMM
        | VectorOp::VMXnorMM => exec_mask_logical(op, vpr, vd, vs2, operand1, ctx),

        // Mask scalar
        VectorOp::VCPopM => exec_vcpop(vpr, vs2, ctx),
        VectorOp::VFirstM => exec_vfirst(vpr, vs2, ctx),

        // Mask-producing
        VectorOp::VMSbfM | VectorOp::VMSifM | VectorOp::VMSofM => {
            exec_mask_set(op, vpr, vd, vs2, ctx)
        }

        // Mask misc
        VectorOp::VIotaM => exec_viota(vpr, vd, vs2, ctx),
        VectorOp::VIdV => exec_vid(vpr, vd, ctx),

        _ => unreachable!("not a mask op: {:?}", op),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Read v0 mask bit for element `i`.
#[inline]
fn mask_active(vpr: &impl VectorRegFile, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Extract the vs1 register index from a [`VecOperand`].
///
/// Mask logical operations always use vector-vector form, so `operand1`
/// must be [`VecOperand::Vector`].
#[inline]
fn vs1_idx(operand1: &VecOperand) -> VRegIdx {
    match operand1 {
        VecOperand::Vector(idx) => *idx,
        _ => unreachable!("mask logical ops require VecOperand::Vector for vs1"),
    }
}

/// Construct a [`VecExecResult`] with no side-effects and no scalar output.
#[inline]
const fn no_result() -> VecExecResult {
    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

/// Construct a [`VecExecResult`] carrying a scalar value.
#[inline]
const fn scalar_result(val: u64) -> VecExecResult {
    VecExecResult { vxsat: false, scalar_result: Some(val), fp_flags: FpFlags::NONE }
}

// ============================================================================
// Mask-register logical
// ============================================================================

/// Compute the logical result for a single mask bit pair.
#[inline]
fn compute_mask_logical(op: VectorOp, s2: bool, s1: bool) -> bool {
    match op {
        VectorOp::VMAndMM => s2 & s1,
        VectorOp::VMNandMM => !(s2 & s1),
        VectorOp::VMAndnMM => s2 && !s1,
        VectorOp::VMOrMM => s2 | s1,
        VectorOp::VMNorMM => !(s2 | s1),
        VectorOp::VMOrnMM => s2 || !s1,
        VectorOp::VMXorMM => s2 ^ s1,
        VectorOp::VMXnorMM => !(s2 ^ s1),
        _ => unreachable!(),
    }
}

/// Execute a mask-register logical operation.
///
/// These operate on individual mask bits for elements `[vstart, vl)`.
/// Mask logical instructions are always unmasked (vm=1 is required by the
/// encoding). Tail bits (`>= vl`) follow the tail-agnostic policy: write 1
/// if [`TailPolicy::Agnostic`].
fn exec_mask_logical(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vs1 = vs1_idx(operand1);
    // Mask registers hold VLEN bits total.
    let vlen_bits = vpr.vlen().bits();

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            // Tail region.
            if ctx.vta.is_agnostic() {
                vpr.write_mask_bit(vd, ElemIdx::new(i), true);
            }
            continue;
        }
        let s2 = vpr.read_mask_bit(vs2, ElemIdx::new(i));
        let s1 = vpr.read_mask_bit(vs1, ElemIdx::new(i));
        let result = compute_mask_logical(op, s2, s1);
        vpr.write_mask_bit(vd, ElemIdx::new(i), result);
    }

    no_result()
}

// ============================================================================
// Mask scalar: vcpop, vfirst
// ============================================================================

/// Execute `vcpop.m` — count set bits in the source mask register.
///
/// Counts mask bits in `vs2` over the range `[vstart, vl)` that are set.
/// When masking is active (`vm=false`), only bits where v0 is also set are
/// counted. The count is returned as a scalar `u64`.
fn exec_vcpop(vpr: &impl VectorRegFile, vs2: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let mut count: u64 = 0;
    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        if vpr.read_mask_bit(vs2, ElemIdx::new(i)) {
            count += 1;
        }
    }
    scalar_result(count)
}

/// Execute `vfirst.m` — find the lowest set bit in the source mask register.
///
/// Scans `vs2` over `[vstart, vl)`. When masking is active (`vm=false`),
/// only positions where v0 is set are considered. Returns the index of the
/// first set bit, or `u64::MAX` (representing -1 in two's complement) if
/// no set bit is found.
fn exec_vfirst(vpr: &impl VectorRegFile, vs2: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    for i in ctx.vstart..ctx.vl {
        if !ctx.vm && !mask_active(vpr, i) {
            continue;
        }
        if vpr.read_mask_bit(vs2, ElemIdx::new(i)) {
            return scalar_result(i as u64);
        }
    }
    scalar_result(u64::MAX)
}

// ============================================================================
// Mask-producing: vmsbf, vmsif, vmsof
// ============================================================================

/// Execute `vmsbf.m`, `vmsif.m`, or `vmsof.m`.
///
/// These instructions scan `vs2` for the first set bit and produce a mask
/// result in `vd`:
/// - **vmsbf**: bits below the first set bit are set; all others cleared.
/// - **vmsif**: bits at and below the first set bit are set; all others cleared.
/// - **vmsof**: only the first set bit is set; all others cleared.
///
/// Inactive elements (when `vm=false` and v0 bit is clear) follow the mask
/// policy. Tail bits follow the tail-agnostic policy.
fn exec_mask_set(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Total mask bits for tail handling.
    let vlen_bits = vpr.vlen().bits();
    // Track whether we have found the first set bit in vs2.
    let mut found_first = false;

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            // Tail region.
            if ctx.vta.is_agnostic() {
                vpr.write_mask_bit(vd, ElemIdx::new(i), true);
            }
            continue;
        }
        // Masking check.
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_mask_bit(vd, ElemIdx::new(i), true);
            }
            continue;
        }

        let src_bit = vpr.read_mask_bit(vs2, ElemIdx::new(i));

        let result = if found_first {
            // After the first set bit: all three output 0.
            false
        } else if src_bit {
            found_first = true;
            match op {
                // sbf: the first set bit itself is NOT included.
                VectorOp::VMSbfM => false,
                // sif: the first set bit IS included.
                // sof: only the first set bit is set.
                VectorOp::VMSifM | VectorOp::VMSofM => true,
                _ => unreachable!(),
            }
        } else {
            // Before the first set bit: sbf and sif output 1, sof outputs 0.
            match op {
                VectorOp::VMSbfM | VectorOp::VMSifM => true,
                VectorOp::VMSofM => false,
                _ => unreachable!(),
            }
        };

        vpr.write_mask_bit(vd, ElemIdx::new(i), result);
    }

    no_result()
}

// ============================================================================
// Mask misc: viota, vid
// ============================================================================

/// Execute `viota.m` — prefix sum of mask bits.
///
/// For each element `i` in `[vstart, vl)`, writes to `vd[i]` (at current
/// SEW) the count of set bits in the source mask `vs2` at positions
/// `[0, i)`. Can be masked by v0.
fn exec_viota(
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    // Pre-compute the running sum of mask bits for positions [0, i).
    // We accumulate as we go rather than pre-building an array, since we
    // need the count *before* position i.
    let mut running_sum: u64 = 0;

    for i in 0..vlmax {
        if i < ctx.vstart {
            // Still accumulate the mask bit for the prefix sum, even in
            // the prestart region, because later elements need the count.
            if vpr.read_mask_bit(vs2, ElemIdx::new(i)) {
                running_sum += 1;
            }
            continue;
        }
        if i >= ctx.vl {
            // Tail region.
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            // No need to keep accumulating past vl.
            continue;
        }

        // The prefix sum for position i is the count of set bits in [0, i),
        // which is `running_sum` before we incorporate bit i.
        let prefix = running_sum;

        // Accumulate bit i for subsequent elements.
        if vpr.read_mask_bit(vs2, ElemIdx::new(i)) {
            running_sum += 1;
        }

        // Masking check.
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, prefix);
    }

    no_result()
}

/// Execute `vid.v` — write element indices.
///
/// For each active element `i` in `[vstart, vl)`, writes `i` to `vd[i]`
/// at the current SEW. Independent of any source register.
fn exec_vid(vpr: &mut impl VectorRegFile, vd: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, i as u64);
    }

    no_result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::arch::vpr::Vpr;
    use crate::core::units::vpu::types::{MaskPolicy, Sew, TailPolicy, Vlen, Vlmul, Vxrm};

    /// Create a 128-bit VPR for testing.
    fn test_vpr() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    /// Build a default execution context.
    fn default_ctx(vl: usize) -> VecExecCtx {
        VecExecCtx {
            sew: Sew::E32,
            vl,
            vstart: 0,
            vma: MaskPolicy::Undisturbed,
            vta: TailPolicy::Undisturbed,
            vlmul: Vlmul::M1,
            vm: true,
            vxrm: Vxrm::RoundToNearestUp,
        }
    }

    // ── Mask logical tests ────────────────────────────────────────────────

    #[test]
    fn test_vmand() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);
        let vs1 = VRegIdx::new(4);

        // vs2: bits 0,1,2,3 set → 0b1111
        // vs1: bits 0,2 set     → 0b0101
        for i in 0..4 {
            vpr.write_mask_bit(vs2, ElemIdx::new(i), true);
        }
        vpr.write_mask_bit(vs1, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs1, ElemIdx::new(2), true);

        let ctx = default_ctx(4);
        let operand = VecOperand::Vector(vs1);
        let _ = vec_mask_execute(VectorOp::VMAndMM, &mut vpr, vd, vs2, &operand, &ctx);

        // Expected: 0 & 0 = 0b0101
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(1)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

    #[test]
    fn test_vmnand() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);
        let vs1 = VRegIdx::new(4);

        // Both set at bit 0 → NAND = false. Neither set at bit 1 → NAND = true.
        vpr.write_mask_bit(vs2, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs1, ElemIdx::new(0), true);

        let ctx = default_ctx(2);
        let operand = VecOperand::Vector(vs1);
        let _ = vec_mask_execute(VectorOp::VMNandMM, &mut vpr, vd, vs2, &operand, &ctx);

        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(1)));
    }

    #[test]
    fn test_vmxnor() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);
        let vs1 = VRegIdx::new(4);

        // Same bits → XNOR = true. Different → XNOR = false.
        vpr.write_mask_bit(vs2, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs1, ElemIdx::new(0), true);
        // bit 1: both false → XNOR = true
        // bit 2: vs2=true, vs1=false → XNOR = false
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);

        let ctx = default_ctx(3);
        let operand = VecOperand::Vector(vs1);
        let _ = vec_mask_execute(VectorOp::VMXnorMM, &mut vpr, vd, vs2, &operand, &ctx);

        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(1)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(2)));
    }

    #[test]
    fn test_mask_logical_tail_agnostic() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);
        let vs1 = VRegIdx::new(4);

        // Pre-fill tail bits with known values
        vpr.write_mask_bit(vd, ElemIdx::new(2), false);
        vpr.write_mask_bit(vd, ElemIdx::new(3), true);

        let mut ctx = default_ctx(2);
        ctx.vta = TailPolicy::Agnostic;

        let operand = VecOperand::Vector(vs1);
        let _ = vec_mask_execute(VectorOp::VMAndMM, &mut vpr, vd, vs2, &operand, &ctx);

        // Tail bits (>= vl=2) should be all-1s when tail-agnostic.
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

    // ── vcpop tests ───────────────────────────────────────────────────────

    #[test]
    fn test_vcpop_unmasked() {
        let mut vpr = test_vpr();
        let vs2 = VRegIdx::new(1);

        vpr.write_mask_bit(vs2, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(3), true);

        let ctx = default_ctx(4);
        let result = exec_vcpop(&vpr, vs2, &ctx);
        assert_eq!(result.scalar_result, Some(3));
    }

    #[test]
    fn test_vcpop_masked() {
        let mut vpr = test_vpr();
        let vs2 = VRegIdx::new(1);
        let v0 = VRegIdx::new(0);

        // vs2: bits 0,1,2 set
        for i in 0..3 {
            vpr.write_mask_bit(vs2, ElemIdx::new(i), true);
        }
        // v0 mask: only bit 0 and 2 active
        vpr.write_mask_bit(v0, ElemIdx::new(0), true);
        vpr.write_mask_bit(v0, ElemIdx::new(2), true);

        let mut ctx = default_ctx(3);
        ctx.vm = false;
        let result = exec_vcpop(&vpr, vs2, &ctx);
        assert_eq!(result.scalar_result, Some(2));
    }

    #[test]
    fn test_vcpop_with_vstart() {
        let mut vpr = test_vpr();
        let vs2 = VRegIdx::new(1);

        // All 4 bits set.
        for i in 0..4 {
            vpr.write_mask_bit(vs2, ElemIdx::new(i), true);
        }

        let mut ctx = default_ctx(4);
        ctx.vstart = 2;
        let result = exec_vcpop(&vpr, vs2, &ctx);
        // Only bits 2,3 counted.
        assert_eq!(result.scalar_result, Some(2));
    }

    // ── vfirst tests ──────────────────────────────────────────────────────

    #[test]
    fn test_vfirst_found() {
        let mut vpr = test_vpr();
        let vs2 = VRegIdx::new(1);

        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);

        let ctx = default_ctx(4);
        let result = exec_vfirst(&vpr, vs2, &ctx);
        assert_eq!(result.scalar_result, Some(2));
    }

    #[test]
    fn test_vfirst_not_found() {
        let vpr = test_vpr();
        let vs2 = VRegIdx::new(1);

        let ctx = default_ctx(4);
        let result = exec_vfirst(&vpr, vs2, &ctx);
        assert_eq!(result.scalar_result, Some(u64::MAX));
    }

    #[test]
    fn test_vfirst_masked() {
        let mut vpr = test_vpr();
        let vs2 = VRegIdx::new(1);
        let v0 = VRegIdx::new(0);

        // vs2: bit 0 and bit 2 set
        vpr.write_mask_bit(vs2, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);
        // v0: only bit 2 active (bit 0 masked off)
        vpr.write_mask_bit(v0, ElemIdx::new(2), true);

        let mut ctx = default_ctx(4);
        ctx.vm = false;
        let result = exec_vfirst(&vpr, vs2, &ctx);
        assert_eq!(result.scalar_result, Some(2));
    }

    // ── vmsbf / vmsif / vmsof tests ───────────────────────────────────────

    #[test]
    fn test_vmsbf() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        // vs2: bit 2 is the first set bit.
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(3), true);

        let ctx = default_ctx(4);
        let _ = exec_mask_set(VectorOp::VMSbfM, &mut vpr, vd, vs2, &ctx);

        // Bits before first (0,1) → set. First (2) → clear. After (3) → clear.
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(1)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

    #[test]
    fn test_vmsif() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);

        let ctx = default_ctx(4);
        let _ = exec_mask_set(VectorOp::VMSifM, &mut vpr, vd, vs2, &ctx);

        // Bits before and including first (0,1,2) → set. After (3) → clear.
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(1)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

    #[test]
    fn test_vmsof() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(3), true);

        let ctx = default_ctx(4);
        let _ = exec_mask_set(VectorOp::VMSofM, &mut vpr, vd, vs2, &ctx);

        // Only the first set bit (2) → set. All others → clear.
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(0)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(1)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(!vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

    #[test]
    fn test_vmsbf_no_set_bit() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        // No bits set in vs2 → all output bits set (before a nonexistent first).
        let ctx = default_ctx(4);
        let _ = exec_mask_set(VectorOp::VMSbfM, &mut vpr, vd, vs2, &ctx);

        for i in 0..4 {
            assert!(vpr.read_mask_bit(vd, ElemIdx::new(i)));
        }
    }

    // ── viota tests ───────────────────────────────────────────────────────

    #[test]
    fn test_viota_basic() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        // vs2 mask: bits 0, 2 set.
        vpr.write_mask_bit(vs2, ElemIdx::new(0), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);

        let ctx = default_ctx(4);
        let _ = exec_viota(&mut vpr, vd, vs2, &ctx);

        // Element 0: count of set bits in [0,0) = 0
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0);
        // Element 1: count of set bits in [0,1) = 1 (bit 0 set)
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 1);
        // Element 2: count of set bits in [0,2) = 1 (bit 0 set)
        assert_eq!(vpr.read_element(vd, ElemIdx::new(2), Sew::E32), 1);
        // Element 3: count of set bits in [0,3) = 2 (bits 0,2 set)
        assert_eq!(vpr.read_element(vd, ElemIdx::new(3), Sew::E32), 2);
    }

    // ── vid tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_vid_basic() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);

        let ctx = default_ctx(4);
        let _ = exec_vid(&mut vpr, vd, &ctx);

        for i in 0..4u64 {
            assert_eq!(vpr.read_element(vd, ElemIdx::new(i as usize), Sew::E32), i);
        }
    }

    #[test]
    fn test_vid_masked() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let v0 = VRegIdx::new(0);

        // Pre-fill vd with 0xFF so we can verify undisturbed elements.
        for i in 0..4 {
            vpr.write_element(vd, ElemIdx::new(i), Sew::E32, 0xFF);
        }
        // v0: only bits 0 and 2 active.
        vpr.write_mask_bit(v0, ElemIdx::new(0), true);
        vpr.write_mask_bit(v0, ElemIdx::new(2), true);

        let mut ctx = default_ctx(4);
        ctx.vm = false;

        let _ = exec_vid(&mut vpr, vd, &ctx);

        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E32), 0);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E32), 0xFF); // undisturbed
        assert_eq!(vpr.read_element(vd, ElemIdx::new(2), Sew::E32), 2);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(3), Sew::E32), 0xFF); // undisturbed
    }

    // ── is_mask_op coverage ───────────────────────────────────────────────

    #[test]
    fn test_is_mask_op() {
        assert!(is_mask_op(VectorOp::VMAndMM));
        assert!(is_mask_op(VectorOp::VMNandMM));
        assert!(is_mask_op(VectorOp::VMAndnMM));
        assert!(is_mask_op(VectorOp::VMOrMM));
        assert!(is_mask_op(VectorOp::VMNorMM));
        assert!(is_mask_op(VectorOp::VMOrnMM));
        assert!(is_mask_op(VectorOp::VMXorMM));
        assert!(is_mask_op(VectorOp::VMXnorMM));
        assert!(is_mask_op(VectorOp::VCPopM));
        assert!(is_mask_op(VectorOp::VFirstM));
        assert!(is_mask_op(VectorOp::VMSbfM));
        assert!(is_mask_op(VectorOp::VMSifM));
        assert!(is_mask_op(VectorOp::VMSofM));
        assert!(is_mask_op(VectorOp::VIotaM));
        assert!(is_mask_op(VectorOp::VIdV));
        // Negative check.
        assert!(!is_mask_op(VectorOp::VAdd));
    }
}
