//! vsetvl/vsetvli/vsetivli execution logic.
//!
//! Implements the vector configuration instructions per RVV 1.0 Section 6.1.

use super::types::{Vlen, Vlmax, encode_vtype, parse_vtype};

/// Execute a vsetvl-family instruction.
///
/// Returns `(new_vl, new_vtype_bits)`.
///
/// # Arguments
///
/// * `avl` — Application Vector Length from rs1 value (or uimm for vsetivli).
/// * `requested_vtype` — Raw vtype bits from rs2 or immediate.
/// * `rd_is_zero` — True if rd == x0.
/// * `rs1_is_zero` — True if rs1 == x0 (for vsetvl/vsetvli; always false for vsetivli).
/// * `vlen` — The configured VLEN for this core.
/// * `current_vl` — Current vl value (used for rd!=x0, rs1==x0 keep-vl case).
pub fn execute_vsetvl(
    avl: u64,
    requested_vtype: u64,
    rd_is_zero: bool,
    rs1_is_zero: bool,
    vlen: Vlen,
    current_vl: u64,
) -> (u64, u64) {
    let fields = parse_vtype(requested_vtype);

    // If vtype is illegal, set vill=1, vl=0
    if fields.vill {
        return (0, 1u64 << 63);
    }

    let vlmax = Vlmax::compute(vlen, fields.vsew, fields.vlmul);
    let new_vtype = encode_vtype(&fields);

    let new_vl = if rd_is_zero && rs1_is_zero {
        // Keep current vl, just change vtype.
        // Per spec: "The vl register is not modified."
        current_vl
    } else if rs1_is_zero {
        // rd != x0, rs1 == x0 → set vl = VLMAX
        vlmax.as_u64()
    } else {
        // Normal case: vl = min(avl, vlmax)
        // This satisfies the spec constraints:
        //   - AVL <= VLMAX → vl = AVL
        //   - AVL > VLMAX → vl = VLMAX (which is >= ceil(AVL/2) when AVL < 2*VLMAX)
        //   - AVL >= 2*VLMAX → vl = VLMAX
        avl.min(vlmax.as_u64())
    };

    (new_vl, new_vtype)
}

#[cfg(test)]
mod tests {
    use super::super::types::{MaskPolicy, Sew, TailPolicy, Vlmul, VtypeFields};
    use super::*;

    fn vlen128() -> Vlen {
        Vlen::new_unchecked(128)
    }

    /// Encode a valid vtype: vlmul=M1, vsew=E32, vta=0, vma=0
    fn vtype_m1_e32() -> u64 {
        encode_vtype(&VtypeFields {
            vsew: Sew::E32,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        })
    }

    /// Encode: vlmul=M8, vsew=E8
    fn vtype_m8_e8() -> u64 {
        encode_vtype(&VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M8,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        })
    }

    #[test]
    fn test_basic_vsetvli() {
        // VLEN=128, SEW=32, LMUL=1 → VLMAX=4
        // AVL=3, rd!=0, rs1!=0
        let (vl, vtype) = execute_vsetvl(3, vtype_m1_e32(), false, false, vlen128(), 0);
        assert_eq!(vl, 3);
        assert_eq!(vtype, vtype_m1_e32());
    }

    #[test]
    fn test_avl_exceeds_vlmax() {
        // VLMAX=4, AVL=10 → vl = min(10, 4) = 4
        let (vl, _) = execute_vsetvl(10, vtype_m1_e32(), false, false, vlen128(), 0);
        assert_eq!(vl, 4);
    }

    #[test]
    fn test_avl_zero() {
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, false, vlen128(), 0);
        assert_eq!(vl, 0);
    }

    #[test]
    fn test_rs1_zero_rd_nonzero_sets_vlmax() {
        // rs1=x0, rd!=x0 → vl = VLMAX = 4
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, true, vlen128(), 0);
        assert_eq!(vl, 4);
    }

    #[test]
    fn test_rd_zero_rs1_zero_keeps_vl() {
        // rd=x0, rs1=x0 → keep current vl
        let (vl, vtype) = execute_vsetvl(0, vtype_m1_e32(), true, true, vlen128(), 7);
        assert_eq!(vl, 7);
        assert_eq!(vtype, vtype_m1_e32());
    }

    #[test]
    fn test_illegal_vtype_sets_vill() {
        // Reserved vlmul=0b100 → vill
        let illegal_vtype = 0b0000_0100_u64;
        let (vl, vtype) = execute_vsetvl(10, illegal_vtype, false, false, vlen128(), 0);
        assert_eq!(vl, 0);
        assert_eq!(vtype, 1u64 << 63);
    }

    #[test]
    fn test_large_vlmax_m8_e8() {
        // VLEN=128, SEW=8, LMUL=8 → VLMAX=128
        let (vl, _) = execute_vsetvl(100, vtype_m8_e8(), false, false, vlen128(), 0);
        assert_eq!(vl, 100);

        let (vl, _) = execute_vsetvl(200, vtype_m8_e8(), false, false, vlen128(), 0);
        assert_eq!(vl, 128);
    }

    #[test]
    fn test_set_vlmax_with_large_vlen() {
        let vlen = Vlen::new_unchecked(256);
        // VLEN=256, SEW=32, LMUL=1 → VLMAX=8
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, true, vlen, 0);
        assert_eq!(vl, 8);
    }

    #[test]
    fn test_sew_too_large_for_fractional_lmul() {
        // vlmul=Mf8, vsew=E64 → SEW > ELEN*LMUL → vill
        let bad_vtype = encode_vtype(&VtypeFields {
            vsew: Sew::E64,
            vlmul: Vlmul::Mf8,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        });
        // This should already have vill set by parse_vtype
        let (vl, vtype) = execute_vsetvl(10, bad_vtype, false, false, vlen128(), 0);
        assert_eq!(vl, 0);
        assert_eq!(vtype, 1u64 << 63);
    }

    #[test]
    fn test_vsetivli_uimm() {
        // vsetivli: rs1_is_zero is always false, avl comes from uimm
        let (vl, _) = execute_vsetvl(15, vtype_m1_e32(), false, false, vlen128(), 0);
        assert_eq!(vl, 4); // VLMAX=4, so min(15,4)=4
    }
}
