//! Vector extension types, newtypes, and vtype CSR parsing.
//!
//! Every domain-specific value in the vector pipeline uses a strict newtype
//! with no implicit conversions. This prevents an entire class of bugs where
//! SEW bytes are confused with SEW bits, element indices with byte offsets,
//! vector register indices with scalar register indices, etc.

// ============================================================================
// Core newtypes
// ============================================================================

/// Vector register index (0–31). NOT interchangeable with `RegIdx` (GPR/FPR).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct VRegIdx(u8);

impl VRegIdx {
    /// Creates a `VRegIdx` from a raw `u8`.
    ///
    /// # Panics
    ///
    /// Panics if `val >= 32`.
    #[inline(always)]
    pub const fn new(val: u8) -> Self {
        assert!(val < 32, "vector register index out of range");
        Self(val)
    }

    /// Returns the raw index.
    #[inline(always)]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns the index as a `usize` for array subscript.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns `true` if this is v0 (mask register).
    #[inline(always)]
    pub const fn is_v0(self) -> bool {
        self.0 == 0
    }

    /// Check if this register is a valid base for an LMUL group.
    #[inline(always)]
    pub const fn is_aligned(self, group: LmulGroup) -> bool {
        self.0.is_multiple_of(group.regs())
    }
}

impl std::fmt::Display for VRegIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

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

/// Effective Element Width for loads/stores (can differ from SEW).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Eew(Sew);

impl Eew {
    /// Create from a `Sew` value.
    #[inline(always)]
    pub const fn new(sew: Sew) -> Self {
        Self(sew)
    }

    /// Returns the underlying `Sew`.
    #[inline(always)]
    pub const fn sew(self) -> Sew {
        self.0
    }

    /// Width in bits.
    #[inline(always)]
    pub const fn bits(self) -> usize {
        self.0.bits()
    }

    /// Width in bytes.
    #[inline(always)]
    pub const fn bytes(self) -> usize {
        self.0.bytes()
    }
}

/// Element index within a vector register (group).
/// Range: 0..VLMAX. NOT interchangeable with plain `usize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ElemIdx(usize);

impl ElemIdx {
    /// Create a new element index.
    #[inline(always)]
    pub const fn new(val: usize) -> Self {
        Self(val)
    }

    /// Returns the index as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Vector length (vl CSR value). NOT interchangeable with `Vlmax` or element count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Vl(u64);

impl Vl {
    /// Create a new vector length value.
    #[inline(always)]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the value as `u64`.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns the value as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns true if the vector length is zero.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

/// VLMAX value (maximum vector length for current vtype). Derived, never directly set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vlmax(usize);

impl Vlmax {
    /// Compute VLMAX = (VLEN / SEW) * LMUL.
    pub const fn compute(vlen: Vlen, sew: Sew, lmul: Vlmul) -> Self {
        let vlen_bits = vlen.bits();
        let sew_bits = sew.bits();
        let (num, den) = lmul.as_fraction();
        // VLMAX = (VLEN / SEW) * (LMUL_num / LMUL_den)
        Self((vlen_bits / sew_bits) * num / den)
    }

    /// Returns the value as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Returns the value as `u64`.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0 as u64
    }
}

/// VLEN (vector register width in bits). Immutable per-core configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vlen(usize);

impl Vlen {
    /// Creates a `Vlen` from a raw value.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the value is not a power of 2 in range [128, 2048].
    pub const fn new(val: usize) -> Result<Self, &'static str> {
        if !val.is_power_of_two() || val < 128 || val > 2048 {
            return Err("VLEN must be power of 2 in range [128, 2048]");
        }
        Ok(Self(val))
    }

    /// Creates a `Vlen` without validation. For use in const contexts where
    /// the value is known valid.
    ///
    /// # Safety (logical)
    ///
    /// Caller must ensure val is a power of 2 in [128, 2048].
    pub const fn new_unchecked(val: usize) -> Self {
        Self(val)
    }

    /// Width in bits.
    #[inline(always)]
    pub const fn bits(self) -> usize {
        self.0
    }

    /// Width in bytes.
    #[inline(always)]
    pub const fn bytes(self) -> usize {
        self.0 / 8
    }
}

/// Number of vector execution lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NumLanes(usize);

impl NumLanes {
    /// # Panics
    ///
    /// Panics if `val` is zero.
    pub const fn new(val: usize) -> Self {
        assert!(val >= 1, "NumLanes must be >= 1");
        Self(val)
    }

    /// Returns the lane count as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Segment field count (nf). Range 1..=8 (encoded as 0..=7 in instruction).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Nf(u8);

impl Nf {
    /// Create from the 3-bit encoded value (0..=7 → fields 1..=8).
    #[inline(always)]
    pub const fn from_encoding(enc: u8) -> Self {
        Self((enc & 0x7) + 1)
    }

    /// Returns the actual field count (1..=8).
    #[inline(always)]
    pub const fn fields(self) -> u8 {
        self.0
    }

    /// Returns the field count as usize.
    #[inline(always)]
    pub const fn fields_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns true if this is a non-segment operation (nf=1).
    #[inline(always)]
    pub const fn is_single(self) -> bool {
        self.0 == 1
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
            vill: true, // invalid by default until configured
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

/// Vector physical register index (O3 backend).
/// NOT interchangeable with `PhysReg` (scalar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VecPhysReg(u16);

impl VecPhysReg {
    /// The zero physical register (no allocation).
    pub const ZERO: Self = Self(0);

    /// Create a new vector physical register index.
    #[inline(always)]
    pub const fn new(val: u16) -> Self {
        Self(val)
    }

    /// Returns the index as `u16`.
    #[inline(always)]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the index as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns true if this is the zero register.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl crate::core::pipeline::free_list::PhysRegister for VecPhysReg {
    #[inline]
    fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[inline]
    fn from_index(idx: u16) -> Self {
        Self(idx)
    }
}

// ============================================================================
// vtype CSR parsing and encoding
// ============================================================================

/// Parse the raw vtype CSR bits into strongly-typed fields.
pub const fn parse_vtype(vtype_bits: u64) -> VtypeFields {
    // vill is the highest bit (bit 63 for RV64)
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
    let vta = if (vtype_bits >> 6) & 1 != 0 {
        TailPolicy::Agnostic
    } else {
        TailPolicy::Undisturbed
    };
    let vma = if (vtype_bits >> 7) & 1 != 0 {
        MaskPolicy::Agnostic
    } else {
        MaskPolicy::Undisturbed
    };

    // Check for invalid encodings
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

    // Check SEW <= LMUL * ELEN constraint
    // For fractional LMUL, SEW must be small enough:
    // SEW <= ELEN * LMUL → SEW * LMUL_den <= ELEN * LMUL_num
    let (lmul_num, lmul_den) = vlmul.as_fraction();
    let sew_bits = vsew.bits();
    // ELEN is 64 for RV64
    if sew_bits * lmul_den > 64 * lmul_num {
        return VtypeFields {
            vsew: Sew::E8,
            vlmul: Vlmul::M1,
            vta: TailPolicy::Undisturbed,
            vma: MaskPolicy::Undisturbed,
            vill: true,
        };
    }

    VtypeFields {
        vsew,
        vlmul,
        vta,
        vma,
        vill: false,
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // --- VRegIdx tests ---

    #[test]
    fn test_vregidx_range() {
        let v0 = VRegIdx::new(0);
        assert!(v0.is_v0());
        assert_eq!(v0.as_u8(), 0);

        let v31 = VRegIdx::new(31);
        assert_eq!(v31.as_u8(), 31);
        assert!(!v31.is_v0());
    }

    #[test]
    #[should_panic(expected = "vector register index out of range")]
    fn test_vregidx_out_of_range() {
        let _ = VRegIdx::new(32);
    }

    #[test]
    fn test_vregidx_alignment() {
        let v0 = VRegIdx::new(0);
        let v1 = VRegIdx::new(1);
        let v4 = VRegIdx::new(4);
        let group4 = Vlmul::M4.group_regs();

        assert!(v0.is_aligned(group4));
        assert!(!v1.is_aligned(group4));
        assert!(v4.is_aligned(group4));
    }

    // --- Sew tests ---

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

    // --- Vlmul tests ---

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

    // --- Vlmax tests ---

    #[test]
    fn test_vlmax_computation() {
        let vlen = Vlen::new_unchecked(128);
        assert_eq!(Vlmax::compute(vlen, Sew::E8, Vlmul::M1).as_usize(), 16);
        assert_eq!(Vlmax::compute(vlen, Sew::E32, Vlmul::M1).as_usize(), 4);
        assert_eq!(Vlmax::compute(vlen, Sew::E8, Vlmul::M8).as_usize(), 128);
        assert_eq!(Vlmax::compute(vlen, Sew::E64, Vlmul::Mf8).as_usize(), 0);

        let vlen256 = Vlen::new_unchecked(256);
        assert_eq!(Vlmax::compute(vlen256, Sew::E32, Vlmul::M2).as_usize(), 16);
    }

    // --- Vlen tests ---

    #[test]
    fn test_vlen_validation() {
        assert!(Vlen::new(128).is_ok());
        assert!(Vlen::new(256).is_ok());
        assert!(Vlen::new(2048).is_ok());
        assert!(Vlen::new(64).is_err());
        assert!(Vlen::new(100).is_err());
        assert!(Vlen::new(4096).is_err());
    }

    // --- Nf tests ---

    #[test]
    fn test_nf() {
        let nf = Nf::from_encoding(0);
        assert_eq!(nf.fields(), 1);
        assert!(nf.is_single());

        let nf7 = Nf::from_encoding(7);
        assert_eq!(nf7.fields(), 8);
        assert!(!nf7.is_single());
    }

    // --- Policy tests ---

    #[test]
    fn test_tail_mask_policy() {
        assert_eq!(TailPolicy::from_bit(false), TailPolicy::Undisturbed);
        assert_eq!(TailPolicy::from_bit(true), TailPolicy::Agnostic);
        assert_eq!(MaskPolicy::from_bit(false), MaskPolicy::Undisturbed);
        assert_eq!(MaskPolicy::from_bit(true), MaskPolicy::Agnostic);
    }

    // --- Vxrm tests ---

    #[test]
    fn test_vxrm_roundtrip() {
        for bits in 0..4u8 {
            let vxrm = Vxrm::from_bits(bits);
            assert_eq!(vxrm.to_bits(), bits);
        }
    }

    // --- vtype parse/encode tests ---

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
