//! The `vtype` CSR: element width, register grouping, tail and mask
//! policies, and the configuration a `vsetvl` establishes.

/// Selected Element Width in bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sew {
    /// 8-bit element width.
    #[default]
    E8,
    /// 16-bit element width.
    E16,
    /// 32-bit element width.
    E32,
    /// 64-bit element width.
    E64,
}

impl Sew {
    /// Element width in bits.
    #[inline(always)]
    pub const fn bits(self) -> usize {
        match self {
            Self::E8 => 8,
            Self::E16 => 16,
            Self::E32 => 32,
            Self::E64 => 64,
        }
    }

    /// Element width in bytes.
    #[inline(always)]
    pub const fn bytes(self) -> usize {
        self.bits() / 8
    }

    /// Bitmask for a single element.
    #[inline(always)]
    pub const fn mask(self) -> u64 {
        match self {
            Self::E8 => 0xFF,
            Self::E16 => 0xFFFF,
            Self::E32 => 0xFFFF_FFFF,
            Self::E64 => u64::MAX,
        }
    }

    /// All-ones fill value for agnostic policy at this width.
    /// Alias for [`mask`](Self::mask) — reads better at call sites.
    #[inline(always)]
    pub const fn ones(self) -> u64 {
        self.mask()
    }

    /// Maximum representable signed value at this SEW (e.g. 127 for E8).
    #[inline(always)]
    pub const fn signed_max(self) -> i64 {
        (1i64 << (self.bits() - 1)) - 1
    }

    /// Minimum representable signed value at this SEW (e.g. -128 for E8).
    #[inline(always)]
    pub const fn signed_min(self) -> i64 {
        -(1i64 << (self.bits() - 1))
    }

    /// Sign-extend a SEW-width value stored in a `u64` to a full `i64`.
    #[inline(always)]
    pub const fn sign_extend(self, val: u64) -> i64 {
        let shift = 64 - self.bits();
        ((val << shift) as i64) >> shift
    }

    /// Decode from the 3-bit vsew encoding.
    #[inline(always)]
    pub const fn from_encoding(enc: u8) -> Option<Self> {
        match enc {
            0 => Some(Self::E8),
            1 => Some(Self::E16),
            2 => Some(Self::E32),
            3 => Some(Self::E64),
            _ => None,
        }
    }

    /// Returns the 3-bit encoding.
    #[inline(always)]
    pub const fn to_encoding(self) -> u8 {
        match self {
            Self::E8 => 0,
            Self::E16 => 1,
            Self::E32 => 2,
            Self::E64 => 3,
        }
    }
}

/// Vector Length Multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vlmul {
    /// LMUL = 1/8
    Mf8,
    /// LMUL = 1/4
    Mf4,
    /// LMUL = 1/2
    Mf2,
    /// LMUL = 1
    M1,
    /// LMUL = 2
    M2,
    /// LMUL = 4
    M4,
    /// LMUL = 8
    M8,
}

impl Vlmul {
    /// Decode from the 3-bit vlmul encoding. Returns None for reserved (0b100).
    #[inline(always)]
    pub const fn from_encoding(enc: u8) -> Option<Self> {
        match enc & 0x7 {
            0b000 => Some(Self::M1),
            0b001 => Some(Self::M2),
            0b010 => Some(Self::M4),
            0b011 => Some(Self::M8),
            0b101 => Some(Self::Mf8),
            0b110 => Some(Self::Mf4),
            0b111 => Some(Self::Mf2),
            _ => None, // 0b100 is reserved
        }
    }

    /// Returns the 3-bit encoding.
    #[inline(always)]
    pub const fn to_encoding(self) -> u8 {
        match self {
            Self::M1 => 0b000,
            Self::M2 => 0b001,
            Self::M4 => 0b010,
            Self::M8 => 0b011,
            Self::Mf8 => 0b101,
            Self::Mf4 => 0b110,
            Self::Mf2 => 0b111,
        }
    }

    /// Returns the physical register group size (1, 2, 4, or 8).
    /// Fractional LMUL uses group size 1.
    #[inline(always)]
    pub const fn group_regs(self) -> LmulGroup {
        match self {
            Self::Mf8 | Self::Mf4 | Self::Mf2 | Self::M1 => LmulGroup(1),
            Self::M2 => LmulGroup(2),
            Self::M4 => LmulGroup(4),
            Self::M8 => LmulGroup(8),
        }
    }

    /// Returns true for fractional LMUL (Mf2/Mf4/Mf8). Distinguishes from
    /// integer LMUL=M1 which `group_regs()` collapses to the same `1` register.
    #[inline(always)]
    pub const fn is_fractional(self) -> bool {
        matches!(self, Self::Mf8 | Self::Mf4 | Self::Mf2)
    }

    /// Returns the register-group size for `EMUL = 2 * LMUL` (widening ops'
    /// destination). For fractional LMUL the doubled EMUL is still ≤ 1
    /// register (e.g. `2 * Mf8 = Mf4`), so this returns 1. For M8 the doubled
    /// EMUL is illegal (16 registers); we cap at 8 here so callers don't
    /// overflow — `vill` should already be set for that case.
    #[inline(always)]
    pub const fn widened_group_regs(self) -> u8 {
        match self {
            Self::Mf8 | Self::Mf4 | Self::Mf2 => 1,
            Self::M1 => 2,
            Self::M2 => 4,
            Self::M4 | Self::M8 => 8,
        }
    }

    /// Returns the LMUL as a (numerator, denominator) fraction.
    #[inline(always)]
    pub const fn as_fraction(self) -> (usize, usize) {
        match self {
            Self::Mf8 => (1, 8),
            Self::Mf4 => (1, 4),
            Self::Mf2 => (1, 2),
            Self::M1 => (1, 1),
            Self::M2 => (2, 1),
            Self::M4 => (4, 1),
            Self::M8 => (8, 1),
        }
    }
}

/// LMUL register group size (1, 2, 4, or 8 consecutive registers).
/// Fractional LMUL uses group size 1. This type represents the physical allocation unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LmulGroup(u8);

impl LmulGroup {
    /// Returns the number of registers in this group.
    #[inline(always)]
    pub const fn regs(self) -> u8 {
        self.0
    }

    /// Returns the number of registers as usize.
    #[inline(always)]
    pub const fn regs_usize(self) -> usize {
        self.0 as usize
    }
}

/// Tail element policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TailPolicy {
    /// Tail elements are preserved (undisturbed).
    #[default]
    Undisturbed,
    /// Tail elements may be overwritten with all-1s.
    Agnostic,
}

impl TailPolicy {
    /// Create from the vta bit (0 = undisturbed, 1 = agnostic).
    #[inline(always)]
    pub const fn from_bit(bit: bool) -> Self {
        if bit { Self::Agnostic } else { Self::Undisturbed }
    }

    /// Returns the vta bit value.
    #[inline(always)]
    pub const fn as_bit(self) -> bool {
        matches!(self, Self::Agnostic)
    }

    /// Returns `true` if tail elements should be overwritten with all-1s.
    #[inline(always)]
    pub const fn is_agnostic(self) -> bool {
        matches!(self, Self::Agnostic)
    }
}

/// Masked-off element policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum MaskPolicy {
    /// Masked-off elements are preserved (undisturbed).
    #[default]
    Undisturbed,
    /// Masked-off elements may be overwritten with all-1s.
    Agnostic,
}

impl MaskPolicy {
    /// Create from the vma bit (0 = undisturbed, 1 = agnostic).
    #[inline(always)]
    pub const fn from_bit(bit: bool) -> Self {
        if bit { Self::Agnostic } else { Self::Undisturbed }
    }

    /// Returns the vma bit value.
    #[inline(always)]
    pub const fn as_bit(self) -> bool {
        matches!(self, Self::Agnostic)
    }

    /// Returns `true` if masked-off elements should be overwritten with all-1s.
    #[inline(always)]
    pub const fn is_agnostic(self) -> bool {
        matches!(self, Self::Agnostic)
    }
}

/// Parsed vtype CSR fields. All fields are strongly typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VtypeFields {
    /// Selected element width.
    pub vsew: Sew,
    /// Vector length multiplier.
    pub vlmul: Vlmul,
    /// Tail element policy.
    pub vta: TailPolicy,
    /// Masked-off element policy.
    pub vma: MaskPolicy,
    /// Illegal vtype flag.
    pub vill: bool,
}

impl Default for VtypeFields {
    fn default() -> Self {
        Self {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        }
    }
}

/// Fixed-point rounding mode (vxrm CSR).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Vxrm {
    /// Round-to-nearest-up (vxrm = 0b00).
    #[default]
    RoundToNearestUp,
    /// Round-to-nearest-even (vxrm = 0b01).
    RoundToNearestEven,
    /// Round-down / truncate (vxrm = 0b10).
    RoundDown,
    /// Round-to-odd (vxrm = 0b11).
    RoundToOdd,
}

impl Vxrm {
    /// Decode from the 2-bit vxrm CSR encoding.
    #[inline(always)]
    pub const fn from_bits(val: u8) -> Self {
        match val & 0x3 {
            0 => Self::RoundToNearestUp,
            1 => Self::RoundToNearestEven,
            2 => Self::RoundDown,
            _ => Self::RoundToOdd,
        }
    }

    /// Encode to the 2-bit vxrm CSR encoding.
    #[inline(always)]
    pub const fn to_bits(self) -> u8 {
        match self {
            Self::RoundToNearestUp => 0,
            Self::RoundToNearestEven => 1,
            Self::RoundDown => 2,
            Self::RoundToOdd => 3,
        }
    }
}

/// Parse the raw vtype CSR bits into strongly-typed fields.
/// Parse vtype bits assuming ELEN=64 (standard RV64V configuration).
pub const fn parse_vtype(vtype_bits: u64) -> VtypeFields {
    parse_vtype_with_elen(vtype_bits, 64)
}

/// Parse vtype bits with an explicit ELEN parameter.
///
/// Use this variant when the core may have ELEN=32 (Zve32x/Zve32f profiles).
/// If `SEW > ELEN` or `SEW * LMUL_den > ELEN * LMUL_num`, vill is set.
pub const fn parse_vtype_with_elen(vtype_bits: u64, elen: usize) -> VtypeFields {
    let vill = (vtype_bits >> 63) & 1 != 0;
    if vill {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    }

    let vlmul_enc = (vtype_bits & 0x7) as u8;
    let vsew_enc = ((vtype_bits >> 3) & 0x7) as u8;
    let vta =
        if (vtype_bits >> 6) & 1 != 0 { TailPolicy::Agnostic } else { TailPolicy::Undisturbed };
    let vma =
        if (vtype_bits >> 7) & 1 != 0 { MaskPolicy::Agnostic } else { MaskPolicy::Undisturbed };

    let Some(vlmul) = Vlmul::from_encoding(vlmul_enc) else {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    };

    let Some(vsew) = Sew::from_encoding(vsew_enc) else {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    };

    if vsew.bits() > elen {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    }

    // SEW <= LMUL * ELEN: SEW * LMUL_den <= ELEN * LMUL_num.
    let (lmul_num, lmul_den) = vlmul.as_fraction();
    let sew_bits = vsew.bits();
    if sew_bits * lmul_den > elen * lmul_num {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    }

    VtypeFields { vsew, vlmul, vta, vma, vill: false }
}

/// Encode vtype fields back to the raw CSR bits.
pub const fn encode_vtype(fields: &VtypeFields) -> u64 {
    if fields.vill {
        return 1u64 << 63;
    }
    let mut val: u64 = 0;
    val |= fields.vlmul.to_encoding() as u64;
    val |= (fields.vsew.to_encoding() as u64) << 3;
    if fields.vta.as_bit() {
        val |= 1 << 6;
    }
    if fields.vma.as_bit() {
        val |= 1 << 7;
    }
    val
}

/// The vector configuration a `vsetvl` establishes: what the front end
/// snapshots for the vector instructions it renames, and what commit
/// writes to the `vtype`, `vl` and `vstart` CSRs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VectorConfig {
    /// `vtype`.
    pub vtype: u64,
    /// `vl`.
    pub vl: u64,
    /// `vstart`.
    pub vstart: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_sew_encoding_roundtrip() {
        for enc in 0..4u8 {
            let sew = Sew::from_encoding(enc).unwrap();
            assert_eq!(sew.to_encoding(), enc);
        }
        assert!(Sew::from_encoding(4).is_none());
    }

    #[test]
    fn test_sew_sizes() {
        assert_eq!(Sew::E8.bits(), 8);
        assert_eq!(Sew::E8.bytes(), 1);
        assert_eq!(Sew::E64.bits(), 64);
        assert_eq!(Sew::E64.bytes(), 8);
    }

    #[test]
    fn test_vlmul_encoding_roundtrip() {
        let cases = [
            (0b000, Vlmul::M1),
            (0b001, Vlmul::M2),
            (0b010, Vlmul::M4),
            (0b011, Vlmul::M8),
            (0b101, Vlmul::Mf8),
            (0b110, Vlmul::Mf4),
            (0b111, Vlmul::Mf2),
        ];
        for (enc, expected) in cases {
            let vlmul = Vlmul::from_encoding(enc).unwrap();
            assert_eq!(vlmul, expected);
            assert_eq!(vlmul.to_encoding(), enc);
        }
        assert!(Vlmul::from_encoding(0b100).is_none());
    }

    #[test]
    fn test_vlmul_group_regs() {
        assert_eq!(Vlmul::Mf8.group_regs().regs(), 1);
        assert_eq!(Vlmul::M1.group_regs().regs(), 1);
        assert_eq!(Vlmul::M2.group_regs().regs(), 2);
        assert_eq!(Vlmul::M4.group_regs().regs(), 4);
        assert_eq!(Vlmul::M8.group_regs().regs(), 8);
    }

    #[test]
    fn test_tail_mask_policy() {
        assert_eq!(TailPolicy::from_bit(false), TailPolicy::Undisturbed);
        assert_eq!(TailPolicy::from_bit(true), TailPolicy::Agnostic);
        assert_eq!(MaskPolicy::from_bit(false), MaskPolicy::Undisturbed);
        assert_eq!(MaskPolicy::from_bit(true), MaskPolicy::Agnostic);
    }

    #[test]
    fn test_vxrm_roundtrip() {
        for bits in 0..4u8 {
            let vxrm = Vxrm::from_bits(bits);
            assert_eq!(vxrm.to_bits(), bits);
        }
    }

    #[test]
    fn test_parse_vtype_basic() {
        // vlmul=M1 (000), vsew=E32 (010), vta=0, vma=0
        let vtype = 0b0001_0000_u64;
        let fields = parse_vtype(vtype);
        assert!(!fields.vill);
        assert_eq!(fields.vlmul, Vlmul::M1);
        assert_eq!(fields.vsew, Sew::E32);
        assert_eq!(fields.vta, TailPolicy::Undisturbed);
        assert_eq!(fields.vma, MaskPolicy::Undisturbed);
    }

    #[test]
    fn test_parse_vtype_with_policies() {
        // vlmul=M2 (001), vsew=E16 (001), vta=1, vma=1
        let vtype = 0b1100_1001_u64;
        let fields = parse_vtype(vtype);
        assert!(!fields.vill);
        assert_eq!(fields.vlmul, Vlmul::M2);
        assert_eq!(fields.vsew, Sew::E16);
        assert_eq!(fields.vta, TailPolicy::Agnostic);
        assert_eq!(fields.vma, MaskPolicy::Agnostic);
    }

    #[test]
    fn test_parse_vtype_vill_set() {
        let vtype = 1u64 << 63;
        let fields = parse_vtype(vtype);
        assert!(fields.vill);
    }

    #[test]
    fn test_parse_vtype_reserved_vlmul() {
        let vtype = 0b0000_0100_u64;
        let fields = parse_vtype(vtype);
        assert!(fields.vill);
    }

    #[test]
    fn test_parse_vtype_reserved_vsew() {
        let vtype = 0b0010_0000_u64;
        let fields = parse_vtype(vtype);
        assert!(fields.vill);
    }

    #[test]
    fn test_parse_vtype_sew_too_large_for_lmul() {
        // vlmul=Mf8 (101), vsew=E64 (011) → SEW=64, LMUL=1/8
        // SEW * den = 64 * 8 = 512 > ELEN * num = 64 * 1 → vill
        let vtype = 0b0001_1101_u64;
        let fields = parse_vtype(vtype);
        assert!(fields.vill);
    }

    #[test]
    fn test_encode_vtype_roundtrip() {
        let fields = VtypeFields {
            vsew: Sew::E32,
            vlmul: Vlmul::M4,
            vta: TailPolicy::Agnostic,
            vma: MaskPolicy::Undisturbed,
            vill: false,
        };
        let encoded = encode_vtype(&fields);
        let decoded = parse_vtype(encoded);
        assert_eq!(fields, decoded);
    }

    #[test]
    fn test_encode_vtype_vill() {
        let fields = VtypeFields { vill: true, ..Default::default() };
        let encoded = encode_vtype(&fields);
        assert_eq!(encoded, 1u64 << 63);
    }
}
