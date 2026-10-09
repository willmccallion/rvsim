//! Vector mask operations.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) mask operations:
//! - Mask-register logical: `vmand`, `vmnand`, `vmandn`, `vmor`, `vmnor`,
//!   `vmorn`, `vmxor`, `vmxnor`
//! - Mask scalar: `vcpop.m`, `vfirst.m`
//! - Mask-producing: `vmsbf.m`, `vmsif.m`, `vmsof.m`
//! - Mask misc: `viota.m`, `vid.v`

use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, mask_active};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::{MaskLogicalOp, MaskOp, MaskSetOp};
use crate::isa::rvv::{ElemIdx, VRegIdx, Vlmax};

/// Execute a mask operation; `vs1` is the second mask of a logical op.
///
/// For scalar-producing ops (`vcpop.m`, `vfirst.m`), the result is returned
/// in [`VecExecResult::scalar_result`]. For vector-producing ops, results
/// are written to `vd` in the VPR.
pub fn vec_mask_execute(
    op: MaskOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        MaskOp::Logical(op) => exec_mask_logical(op, vpr, vd, vs2, vs1, ctx),
        MaskOp::CPop => exec_vcpop(vpr, vs2, ctx),
        MaskOp::First => exec_vfirst(vpr, vs2, ctx),
        MaskOp::Set(op) => exec_mask_set(op, vpr, vd, vs2, ctx),
        MaskOp::Iota => exec_viota(vpr, vd, vs2, ctx),
        MaskOp::Id => exec_vid(vpr, vd, ctx),
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

/// Compute the logical result for a single mask bit pair.
#[inline]
const fn compute_mask_logical(op: MaskLogicalOp, s2: bool, s1: bool) -> bool {
    match op {
        MaskLogicalOp::And => s2 & s1,
        MaskLogicalOp::Nand => !(s2 & s1),
        MaskLogicalOp::AndNot => s2 && !s1,
        MaskLogicalOp::Or => s2 | s1,
        MaskLogicalOp::Nor => !(s2 | s1),
        MaskLogicalOp::OrNot => s2 || !s1,
        MaskLogicalOp::Xor => s2 ^ s1,
        MaskLogicalOp::Xnor => !(s2 ^ s1),
    }
}

/// Execute a mask-register logical operation.
///
/// These operate on individual mask bits for elements `[vstart, vl)`.
/// Mask logical instructions are always unmasked (vm=1 is required by the
/// encoding). Tail bits (`>= vl`) follow the tail-agnostic policy: write 1
/// if [`TailPolicy::Agnostic`].
fn exec_mask_logical(
    op: MaskLogicalOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    vs1: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlen_bits = vpr.vlen().bits();

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.fill_agnostic_mask_bit(vd, ElemIdx::new(i));
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
    op: MaskSetOp,
    vpr: &mut impl VectorRegFile,
    vd: VRegIdx,
    vs2: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlen_bits = vpr.vlen().bits();
    let mut found_first = false;

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.fill_agnostic_mask_bit(vd, ElemIdx::new(i));
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.fill_agnostic_mask_bit(vd, ElemIdx::new(i));
            }
            continue;
        }

        let src_bit = vpr.read_mask_bit(vs2, ElemIdx::new(i));

        let result = if found_first {
            false
        } else if src_bit {
            found_first = true;
            match op {
                MaskSetOp::BeforeFirst => false,
                MaskSetOp::IncludingFirst | MaskSetOp::OnlyFirst => true,
            }
        } else {
            match op {
                MaskSetOp::BeforeFirst | MaskSetOp::IncludingFirst => true,
                MaskSetOp::OnlyFirst => false,
            }
        };

        vpr.write_mask_bit(vd, ElemIdx::new(i), result);
    }

    no_result()
}

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

    let mut running_sum: u64 = 0;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.fill_agnostic_element(vd, ElemIdx::new(i), ctx.sew);
            }
            continue;
        }

        let active = ctx.vm || mask_active(vpr, i);
        let prefix = running_sum;

        // RVV 1.0 §15.5: only mask bits at active element positions
        // contribute to the running sum — masked-off positions are skipped.
        if active && vpr.read_mask_bit(vs2, ElemIdx::new(i)) {
            running_sum += 1;
        }

        if !active {
            if ctx.vma.is_agnostic() {
                vpr.fill_agnostic_element(vd, ElemIdx::new(i), ctx.sew);
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
                vpr.fill_agnostic_element(vd, ElemIdx::new(i), ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.fill_agnostic_element(vd, ElemIdx::new(i), ctx.sew);
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
    use crate::isa::op::{VecClass, VectorOp};

    /// The mask operation `op` decodes to.
    fn mask_op(op: VectorOp) -> MaskOp {
        match op.class() {
            VecClass::Mask(mask) => mask,
            other => panic!("{op:?} is not a mask op: {other:?}"),
        }
    }
    use crate::arch::regs::vpr::Vpr;
    use crate::isa::fp::RoundingMode;
    use crate::isa::rvv::{MaskPolicy, Sew, TailPolicy, Vlen, Vlmul, Vxrm};

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
            frm: RoundingMode::Rne,
            zvfh: false,
        }
    }

    #[test]
    fn test_vid_mf8_e8() {
        // vid.v at SEW=E8, LMUL=mf8: VLMAX = (128/8)*1/8 = 2. With vl=2,
        // expect vd[0]=0, vd[1]=1, and bytes 2..16 of vd preserved.
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        // Pre-fill v2 with sentinel.
        for i in 0..16usize {
            vpr.write_element(vd, ElemIdx::new(i), Sew::E8, 0xAA);
        }
        let ctx = VecExecCtx {
            sew: Sew::E8,
            vl: 2,
            vstart: 0,
            vma: MaskPolicy::Undisturbed,
            vta: TailPolicy::Undisturbed,
            vlmul: Vlmul::Mf8,
            vm: true,
            vxrm: Vxrm::RoundToNearestUp,
            frm: RoundingMode::Rne,
            zvfh: false,
        };
        let _ = exec_vid(&mut vpr, vd, &ctx);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(0), Sew::E8), 0);
        assert_eq!(vpr.read_element(vd, ElemIdx::new(1), Sew::E8), 1);
        for i in 2..16 {
            assert_eq!(
                vpr.read_element(vd, ElemIdx::new(i), Sew::E8),
                0xAA,
                "tail byte {i} should be preserved (tu)"
            );
        }
    }

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
        let _ = vec_mask_execute(mask_op(VectorOp::VMAndMM), &mut vpr, vd, vs2, vs1, &ctx);

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
        let _ = vec_mask_execute(mask_op(VectorOp::VMNandMM), &mut vpr, vd, vs2, vs1, &ctx);

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
        let _ = vec_mask_execute(mask_op(VectorOp::VMXnorMM), &mut vpr, vd, vs2, vs1, &ctx);

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

        let _ = vec_mask_execute(mask_op(VectorOp::VMAndMM), &mut vpr, vd, vs2, vs1, &ctx);

        // Tail bits (>= vl=2) should be all-1s when tail-agnostic.
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(2)));
        assert!(vpr.read_mask_bit(vd, ElemIdx::new(3)));
    }

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

    #[test]
    fn test_vmsbf() {
        let mut vpr = test_vpr();
        let vd = VRegIdx::new(2);
        let vs2 = VRegIdx::new(3);

        // vs2: bit 2 is the first set bit.
        vpr.write_mask_bit(vs2, ElemIdx::new(2), true);
        vpr.write_mask_bit(vs2, ElemIdx::new(3), true);

        let ctx = default_ctx(4);
        let _ = exec_mask_set(MaskSetOp::BeforeFirst, &mut vpr, vd, vs2, &ctx);

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
        let _ = exec_mask_set(MaskSetOp::IncludingFirst, &mut vpr, vd, vs2, &ctx);

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
        let _ = exec_mask_set(MaskSetOp::OnlyFirst, &mut vpr, vd, vs2, &ctx);

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
        let _ = exec_mask_set(MaskSetOp::BeforeFirst, &mut vpr, vd, vs2, &ctx);

        for i in 0..4 {
            assert!(vpr.read_mask_bit(vd, ElemIdx::new(i)));
        }
    }

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
}
