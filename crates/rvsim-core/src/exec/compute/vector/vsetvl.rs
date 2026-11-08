//! vsetvl/vsetvli/vsetivli execution logic.
//!
//! Implements the vector configuration instructions per RVV 1.0 Section 6.1.

use crate::isa::rvv::{Vlen, Vlmax, encode_vtype, parse_vtype_with_elen};

/// The `vtype` bits RVV 1.0 reserves, between `vma` and `vill`; a value
/// setting any of them is unsupported.
const VTYPE_RESERVED: u64 = ((1 << 63) - 1) & !0xFF;

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
/// * `elen` — The configured ELEN, the widest SEW it supports.
/// * `current_vl` — Current vl value (used for rd!=x0, rs1==x0 keep-vl case).
pub fn execute_vsetvl(
    avl: u64,
    requested_vtype: u64,
    rd_is_zero: bool,
    rs1_is_zero: bool,
    vlen: Vlen,
    elen: usize,
    current_vl: u64,
) -> (u64, u64) {
    let fields = parse_vtype_with_elen(requested_vtype, elen);

    if fields.vill || requested_vtype & VTYPE_RESERVED != 0 {
        return (0, 1u64 << 63);
    }

    let vlmax = Vlmax::compute(vlen, fields.vsew, fields.vlmul);
    let new_vtype = encode_vtype(&fields);

    let new_vl = if rd_is_zero && rs1_is_zero {
        current_vl
    } else if rs1_is_zero {
        vlmax.as_u64()
    } else {
        avl.min(vlmax.as_u64())
    };

    (new_vl, new_vtype)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::rvv::{MaskPolicy, Sew, TailPolicy, Vlmul, VtypeFields};

    const ELEN: usize = 64;

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
    fn an_avl_below_vlmax_becomes_vl_and_vtype_is_kept() {
        // VLEN=128, SEW=32, LMUL=1 → VLMAX=4
        // AVL=3, rd!=0, rs1!=0
        let (vl, vtype) = execute_vsetvl(3, vtype_m1_e32(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 3);
        assert_eq!(vtype, vtype_m1_e32());
    }

    #[test]
    fn an_avl_above_vlmax_is_clamped_to_vlmax() {
        // VLMAX=4, AVL=10 → vl = min(10, 4) = 4
        let (vl, _) = execute_vsetvl(10, vtype_m1_e32(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 4);
    }

    #[test]
    fn a_zero_avl_gives_a_zero_vl() {
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 0);
    }

    #[test]
    fn rs1_x0_with_rd_nonzero_sets_vl_to_vlmax() {
        // rs1=x0, rd!=x0 → vl = VLMAX = 4
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, true, vlen128(), ELEN, 0);
        assert_eq!(vl, 4);
    }

    #[test]
    fn rd_and_rs1_x0_keep_the_current_vl() {
        // rd=x0, rs1=x0 → keep current vl
        let (vl, vtype) = execute_vsetvl(0, vtype_m1_e32(), true, true, vlen128(), ELEN, 7);
        assert_eq!(vl, 7);
        assert_eq!(vtype, vtype_m1_e32());
    }

    #[test]
    fn a_reserved_vlmul_sets_vill_and_zeroes_vl() {
        // Reserved vlmul=0b100 → vill
        let illegal_vtype = 0b0000_0100_u64;
        let (vl, vtype) = execute_vsetvl(10, illegal_vtype, false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 0);
        assert_eq!(vtype, 1u64 << 63);
    }

    #[test]
    fn e8_m8_at_vlen_128_clamps_vl_to_128() {
        // VLEN=128, SEW=8, LMUL=8 → VLMAX=128
        let (vl, _) = execute_vsetvl(100, vtype_m8_e8(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 100);

        let (vl, _) = execute_vsetvl(200, vtype_m8_e8(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 128);
    }

    #[test]
    fn rs1_x0_sets_vl_to_vlmax_at_vlen_256() {
        let vlen = Vlen::new_unchecked(256);
        // VLEN=256, SEW=32, LMUL=1 → VLMAX=8
        let (vl, _) = execute_vsetvl(0, vtype_m1_e32(), false, true, vlen, ELEN, 0);
        assert_eq!(vl, 8);
    }

    #[test]
    fn e64_with_mf8_sets_vill() {
        // vlmul=Mf8, vsew=E64 → SEW > ELEN*LMUL → vill
        let bad_vtype = encode_vtype(&VtypeFields {
            vsew: Sew::E64,
            vlmul: Vlmul::Mf8,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        });
        // This should already have vill set by parse_vtype
        let (vl, vtype) = execute_vsetvl(10, bad_vtype, false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 0);
        assert_eq!(vtype, 1u64 << 63);
    }

    #[test]
    fn e32_m8_is_legal_and_takes_a_smaller_avl() {
        // AVL=4, e32/m8: VLMAX=(128/32)*8=32, vl=min(4,32)=4
        let bad_vtype = encode_vtype(&VtypeFields {
            vsew: Sew::E32,
            vlmul: Vlmul::M8,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        });
        let (vl, vtype) = execute_vsetvl(4, bad_vtype, false, false, vlen128(), ELEN, 0);
        assert!(vtype & (1u64 << 63) == 0, "vill must not be set: vtype=0x{vtype:x}");
        assert_eq!(vl, 4, "expected vl=4 for AVL=4 e32/m8");
    }

    #[test]
    fn e8_mf8_is_legal_with_a_vlmax_of_two() {
        // vsetvli e8, mf8 — encoding bit5:3=vsew=0(E8), bit2:0=vlmul=0b101(Mf8)
        // This is a LEGAL configuration: SEW * lmul_den (8*8=64) <= ELEN * lmul_num (64*1=64).
        // VLMAX = (128/8)*1/8 = 2
        let vtype_bits: u64 = 0b101; // vsew=0, vlmul=0b101, vta=0, vma=0
        let parsed = crate::isa::rvv::parse_vtype(vtype_bits);
        assert!(!parsed.vill, "mf8/e8 should NOT set vill");
        assert_eq!(parsed.vlmul, crate::isa::rvv::Vlmul::Mf8);
        assert_eq!(parsed.vsew, crate::isa::rvv::Sew::E8);

        // execute_vsetvl with AVL=0, rd!=0, rs1!=0
        let (vl, vtype) = execute_vsetvl(0, vtype_bits, false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 0);
        assert_eq!(vtype & (1u64 << 63), 0, "vill bit must not be set");

        // AVL=12 → vl = min(12, VLMAX=2) = 2
        let (vl, _) = execute_vsetvl(12, vtype_bits, false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 2);
    }

    #[test]
    fn an_immediate_avl_above_vlmax_is_clamped_to_vlmax() {
        // vsetivli: rs1_is_zero is always false, avl comes from uimm
        let (vl, _) = execute_vsetvl(15, vtype_m1_e32(), false, false, vlen128(), ELEN, 0);
        assert_eq!(vl, 4); // VLMAX=4, so min(15,4)=4
    }

    #[test]
    fn a_reserved_vtype_bit_sets_vill() {
        let reserved = vtype_m1_e32() | 1 << 8;

        let (vl, vtype) = execute_vsetvl(10, reserved, false, false, vlen128(), ELEN, 0);

        assert_eq!((vl, vtype), (0, 1u64 << 63));
    }

    #[test]
    fn sew_wider_than_elen_sets_vill() {
        let e64 = encode_vtype(&VtypeFields {
            vsew: Sew::E64,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        });

        let (vl, vtype) = execute_vsetvl(10, e64, false, false, vlen128(), 32, 0);

        assert_eq!((vl, vtype), (0, 1u64 << 63));
    }
}
