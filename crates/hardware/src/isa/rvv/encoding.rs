//! Vector-specific instruction field extraction.

/// Extract the vm bit (bit 25): 0 = masked, 1 = unmasked.
#[inline(always)]
pub const fn vm(inst: u32) -> bool {
    (inst >> 25) & 1 != 0
}

/// Extract nf (bits 31:29): number of fields minus 1 for segment loads/stores.
#[inline(always)]
pub const fn nf(inst: u32) -> u8 {
    ((inst >> 29) & 0x7) as u8
}

/// Extract mop (bits 27:26): memory addressing mode.
/// 00 = unit-stride, 01 = indexed-unordered, 10 = strided, 11 = indexed-ordered.
#[inline(always)]
pub const fn mop(inst: u32) -> u8 {
    ((inst >> 26) & 0x3) as u8
}

/// Extract mew bit (bit 28): extended memory element width (must be 0 for RVV 1.0).
#[inline(always)]
pub const fn mew(inst: u32) -> bool {
    (inst >> 28) & 1 != 0
}

/// Extract lumop (bits 24:20): unit-stride load sub-variant.
/// 00000 = unit-stride, 01000 = whole-register, 01011 = mask, 10000 = fault-only-first.
#[inline(always)]
pub const fn lumop(inst: u32) -> u8 {
    ((inst >> 20) & 0x1F) as u8
}

/// Extract sumop (bits 24:20): unit-stride store sub-variant.
#[inline(always)]
pub const fn sumop(inst: u32) -> u8 {
    ((inst >> 20) & 0x1F) as u8
}

/// Extract funct6 (bits 31:26): vector arithmetic operation code.
#[inline(always)]
pub const fn funct6(inst: u32) -> u32 {
    (inst >> 26) & 0x3F
}

/// Extract vs1 (bits 19:15): vector source register 1 (same position as rs1).
#[inline(always)]
pub const fn vs1(inst: u32) -> u8 {
    ((inst >> 15) & 0x1F) as u8
}

/// Extract vs2 (bits 24:20): vector source register 2 (same position as rs2).
#[inline(always)]
pub const fn vs2(inst: u32) -> u8 {
    ((inst >> 20) & 0x1F) as u8
}

/// Extract vd (bits 11:7): vector destination register (same position as rd).
#[inline(always)]
pub const fn vd(inst: u32) -> u8 {
    ((inst >> 7) & 0x1F) as u8
}

/// Extract simm5 (bits 19:15): sign-extended 5-bit immediate for OPIVI.
#[inline(always)]
pub const fn simm5(inst: u32) -> i64 {
    let raw = ((inst >> 15) & 0x1F) as i32;
    // Sign-extend from 5 bits
    ((raw << 27) >> 27) as i64
}

/// Extract zimm for vsetvli (bits 30:20): 11-bit unsigned immediate.
#[inline(always)]
pub const fn zimm_vsetvli(inst: u32) -> u64 {
    ((inst >> 20) & 0x7FF) as u64
}

/// Extract zimm for vsetivli (bits 29:20): 10-bit unsigned immediate.
#[inline(always)]
pub const fn zimm_vsetivli(inst: u32) -> u64 {
    ((inst >> 20) & 0x3FF) as u64
}

/// Extract uimm for vsetivli (bits 19:15): 5-bit unsigned AVL immediate.
#[inline(always)]
pub const fn uimm_vsetivli(inst: u32) -> u64 {
    ((inst >> 15) & 0x1F) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_bit() {
        assert!(vm(1 << 25));
        assert!(!vm(0));
    }

    #[test]
    fn test_nf() {
        // nf = 3 → bits 31:29 = 011
        let inst = 0b0110_0000_0000_0000_0000_0000_0000_0000_u32;
        assert_eq!(nf(inst), 3);
    }

    #[test]
    fn test_funct6() {
        // funct6 = 0b101010 → bits 31:26
        let inst = 0b1010_1000_0000_0000_0000_0000_0000_0000_u32;
        assert_eq!(funct6(inst), 0b101010);
    }

    #[test]
    fn test_simm5_positive() {
        // simm5 = 7 → bits 19:15 = 00111
        let inst = 0b0000_0000_0000_0011_1000_0000_0000_0000_u32;
        assert_eq!(simm5(inst), 7);
    }

    #[test]
    fn test_simm5_negative() {
        // simm5 = -1 → bits 19:15 = 11111
        let inst = 0b0000_0000_0000_1111_1000_0000_0000_0000_u32;
        assert_eq!(simm5(inst), -1);
    }

    #[test]
    fn test_simm5_min() {
        // simm5 = -16 → bits 19:15 = 10000
        let inst = 0b0000_0000_0000_1000_0000_0000_0000_0000_u32;
        assert_eq!(simm5(inst), -16);
    }

    #[test]
    fn test_vd_vs1_vs2() {
        // vd=5, vs1=10, vs2=15
        let inst = (15u32 << 20) | (10u32 << 15) | (5u32 << 7);
        assert_eq!(vd(inst), 5);
        assert_eq!(vs1(inst), 10);
        assert_eq!(vs2(inst), 15);
    }

    #[test]
    fn test_zimm_vsetvli() {
        let inst = 0x7FF << 20; // all 11 bits set
        assert_eq!(zimm_vsetvli(inst), 0x7FF);
    }

    #[test]
    fn test_zimm_vsetivli() {
        let inst = 0x3FF << 20; // all 10 bits set
        assert_eq!(zimm_vsetivli(inst), 0x3FF);
    }

    #[test]
    fn test_uimm_vsetivli() {
        let inst = 0x1F << 15; // all 5 bits set
        assert_eq!(uimm_vsetivli(inst), 31);
    }
}
