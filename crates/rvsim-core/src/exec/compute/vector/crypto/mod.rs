//! Vector cryptography extensions: Zvkned (AES), Zvknha/b (SHA-2), Zvksed
//! (SM4), Zvksh (SM3), Zvkg (GHASH).
//!
//! Per the RISC-V Vector Crypto specification (v1.0.0), all crypto ops
//! operate on **element groups** (EGS) at SEW=32:
//!
//! | Extension | EGS | SEW |
//! |-----------|-----|-----|
//! | Zvkned    | 4   | 32  |  AES (128-bit state per group)
//! | Zvknha    | 4   | 32  |  SHA-256 (4×32 = 128-bit state)
//! | Zvknhb    | 4   | 32  |  SHA-256/512 (SEW=64 → 4×64 = 256-bit state)
//! | Zvksed    | 4   | 32  |  SM4 (128-bit state)
//! | Zvksh     | 8   | 32  |  SM3 (256-bit state)
//! | Zvkg      | 4   | 32  |  GHASH (128-bit state)
//!
//! For `vl` < EGS, the instruction is treated as a no-op for that group
//! (per spec: "the instruction does not change the destination register
//! group if vl < EGS").
//!
//! For each EGS-sized group, the operands are loaded as 128- or 256-bit
//! state values, the round function is applied, and the result is stored
//! back. With LMUL>1, multiple groups are processed sequentially.

mod aes;
mod ghash;
mod sha2;
mod sm3;
mod sm4;

use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx};
use aes::{
    aes_kf1, aes_kf2, aes_round_dec, aes_round_dec_final, aes_round_enc, aes_round_enc_final,
    aes_round_zero,
};
use ghash::gf128_mul;
use sha2::{sha256_compress_high, sha256_compress_low, sha256_ms};
use sm3::{sm3_c, sm3_me};
use sm4::{sm4_k, sm4_r};

/// Element group size for AES/GHASH/SM4 (4 × SEW=32 = 128 bits).
const EGS_AES: usize = 4;

/// Element group size for SM3 (8 × SEW=32 = 256 bits).
const EGS_SM3: usize = 8;

/// Element group size for SHA-2 (4 × SEW=32 = 128 bits, also 4 × SEW=64 = 256 bits).
const EGS_SHA: usize = 4;

/// Returns true if `op` is a vector crypto instruction handled in this module.
#[allow(clippy::module_name_repetitions)]
pub const fn is_crypto(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VAesEm
            | VectorOp::VAesEf
            | VectorOp::VAesDm
            | VectorOp::VAesDf
            | VectorOp::VAesZ
            | VectorOp::VAesKf1
            | VectorOp::VAesKf2
            | VectorOp::VSha2Ms
            | VectorOp::VSha2Ch
            | VectorOp::VSha2Cl
            | VectorOp::VSm3Me
            | VectorOp::VSm3C
            | VectorOp::VSm4R
            | VectorOp::VSm4K
            | VectorOp::VGhsh
            | VectorOp::VGmul
    )
}

/// Read four 32-bit elements starting at `base_elem` into a `[u32; 4]` array.
#[inline]
fn read_egs4_u32(vpr: &impl VectorRegFile, vreg: VRegIdx, base_elem: usize) -> [u32; 4] {
    let mut out = [0u32; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = vpr.read_element(vreg, ElemIdx::new(base_elem + i), Sew::E32) as u32;
    }
    out
}

/// Write four 32-bit elements starting at `base_elem`.
#[inline]
fn write_egs4_u32(vpr: &mut impl VectorRegFile, vreg: VRegIdx, base_elem: usize, vals: [u32; 4]) {
    for (i, v) in vals.iter().enumerate() {
        vpr.write_element(vreg, ElemIdx::new(base_elem + i), Sew::E32, u64::from(*v));
    }
}

/// Read eight 32-bit elements (used by SM3).
#[inline]
fn read_egs8_u32(vpr: &impl VectorRegFile, vreg: VRegIdx, base_elem: usize) -> [u32; 8] {
    let mut out = [0u32; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = vpr.read_element(vreg, ElemIdx::new(base_elem + i), Sew::E32) as u32;
    }
    out
}

/// Write eight 32-bit elements.
#[inline]
fn write_egs8_u32(vpr: &mut impl VectorRegFile, vreg: VRegIdx, base_elem: usize, vals: [u32; 8]) {
    for (i, v) in vals.iter().enumerate() {
        vpr.write_element(vreg, ElemIdx::new(base_elem + i), Sew::E32, u64::from(*v));
    }
}

/// Execute a vector crypto instruction. Iterates over each EGS-sized element
/// group within `[vstart, vl)` and applies the round function.
///
/// `broadcast_vs2` is set for the `.vs` forms (vaes*/vsm4r .vs, vaesz),
/// which use vs2 element group 0 for every destination element group
/// instead of a per-group key.
///
/// Returns nothing; results are written directly to `vd` in `vpr`.
#[allow(clippy::too_many_arguments)]
pub fn execute_crypto(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    vs1_idx: VRegIdx,
    vstart: usize,
    vl: usize,
    inst: u32,
    broadcast_vs2: bool,
) {
    let egs = if matches!(op, VectorOp::VSm3Me | VectorOp::VSm3C) { EGS_SM3 } else { EGS_AES };
    let _ = EGS_SHA;

    if vl < egs {
        return;
    }

    // .vs form: vs2 element group 0 is broadcast across all destination groups.
    let key_base = |base: usize| if broadcast_vs2 { 0 } else { base };

    let mut base = (vstart / egs) * egs;
    while base + egs <= vl {
        match op {
            VectorOp::VAesEm => {
                let state = read_egs4_u32(vpr, vd_idx, base);
                let key = read_egs4_u32(vpr, vs2_idx, key_base(base));
                let r = aes_round_enc(state, key);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesEf => {
                let state = read_egs4_u32(vpr, vd_idx, base);
                let key = read_egs4_u32(vpr, vs2_idx, key_base(base));
                let r = aes_round_enc_final(state, key);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesDm => {
                let state = read_egs4_u32(vpr, vd_idx, base);
                let key = read_egs4_u32(vpr, vs2_idx, key_base(base));
                let r = aes_round_dec(state, key);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesDf => {
                let state = read_egs4_u32(vpr, vd_idx, base);
                let key = read_egs4_u32(vpr, vs2_idx, key_base(base));
                let r = aes_round_dec_final(state, key);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesZ => {
                // VAesZ is .vs-only per Zvkned, so vs2 always reads element 0.
                let state = read_egs4_u32(vpr, vd_idx, base);
                let key = read_egs4_u32(vpr, vs2_idx, 0);
                let r = aes_round_zero(state, key);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesKf1 => {
                // zimm5 = vs1 field as round number (1..10 valid).
                let rnd = (inst >> 15) & 0x1f;
                let prev = read_egs4_u32(vpr, vs2_idx, base);
                let r = aes_kf1(prev, rnd);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VAesKf2 => {
                let rnd = (inst >> 15) & 0x1f;
                let curr = read_egs4_u32(vpr, vd_idx, base);
                let prev = read_egs4_u32(vpr, vs2_idx, base);
                let r = aes_kf2(curr, prev, rnd);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSha2Ms => {
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, base);
                let vs1 = read_egs4_u32(vpr, vs1_idx, base);
                let r = sha256_ms(vd, vs2, vs1);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSha2Cl => {
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, base);
                let vs1 = read_egs4_u32(vpr, vs1_idx, base);
                let r = sha256_compress_low(vd, vs2, vs1);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSha2Ch => {
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, base);
                let vs1 = read_egs4_u32(vpr, vs1_idx, base);
                let r = sha256_compress_high(vd, vs2, vs1);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSm3Me => {
                let vs1 = read_egs8_u32(vpr, vs1_idx, base);
                let vs2 = read_egs8_u32(vpr, vs2_idx, base);
                let r = sm3_me(vs1, vs2);
                write_egs8_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSm3C => {
                let rnd = (inst >> 15) & 0x1f;
                let vd = read_egs8_u32(vpr, vd_idx, base);
                let vs2 = read_egs8_u32(vpr, vs2_idx, base);
                let r = sm3_c(vd, vs2, rnd);
                write_egs8_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSm4R => {
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, key_base(base));
                let r = sm4_r(vd, vs2);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VSm4K => {
                let rnd = (inst >> 15) & 0x1f;
                let prev = read_egs4_u32(vpr, vs2_idx, base);
                let r = sm4_k(prev, rnd);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VGmul => {
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, base);
                let r = gf128_mul(vd, vs2);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            VectorOp::VGhsh => {
                // Per Zvkg: vd = (vd ^ vs1) * vs2  (Y partial-hash, X cipher
                // output, H subkey).
                let vd = read_egs4_u32(vpr, vd_idx, base);
                let vs2 = read_egs4_u32(vpr, vs2_idx, base);
                let vs1 = read_egs4_u32(vpr, vs1_idx, base);
                let xored = [vd[0] ^ vs1[0], vd[1] ^ vs1[1], vd[2] ^ vs1[2], vd[3] ^ vs1[3]];
                let r = gf128_mul(xored, vs2);
                write_egs4_u32(vpr, vd_idx, base, r);
            }
            _ => unreachable!("execute_crypto called with non-crypto op {:?}", op),
        }
        base += egs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::aes_kf1;
    use aes::aes_round_enc;

    /// Pack 16 bytes (column-major state) into 4 little-endian u32 words.
    fn bytes_to_words(b: [u8; 16]) -> [u32; 4] {
        [
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
        ]
    }

    #[test]
    fn aes_round_enc_matches_fips197_round1() {
        // FIPS 197 Appendix B test vector: AES-128.
        // After round-0 AddRoundKey:
        let state_in = bytes_to_words([
            0x19, 0x3d, 0xe3, 0xbe, 0xa0, 0xf4, 0xe2, 0x2b, 0x9a, 0xc6, 0x8d, 0x2a, 0xe9, 0xf8,
            0x48, 0x08,
        ]);
        // Round 1 expanded key (W[4..8]):
        let round_key = bytes_to_words([
            0xa0, 0xfa, 0xfe, 0x17, 0x88, 0x54, 0x2c, 0xb1, 0x23, 0xa3, 0x39, 0x39, 0x2a, 0x6c,
            0x76, 0x05,
        ]);
        // Round-1 output (state after MixColumns + AddRoundKey):
        let expected = bytes_to_words([
            0xa4, 0x9c, 0x7f, 0xf2, 0x68, 0x9f, 0x35, 0x2b, 0x6b, 0x5b, 0xea, 0x43, 0x02, 0x6a,
            0x50, 0x49,
        ]);
        assert_eq!(aes_round_enc(state_in, round_key), expected);
    }

    #[test]
    fn aes_kf1_matches_fips197_round1() {
        // FIPS 197 key 0x2b7e1516..., round 0 is the original key.
        // Expected round-1 expansion (W[4..8]) from key schedule.
        let key0 = bytes_to_words([
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ]);
        let expected = bytes_to_words([
            0xa0, 0xfa, 0xfe, 0x17, 0x88, 0x54, 0x2c, 0xb1, 0x23, 0xa3, 0x39, 0x39, 0x2a, 0x6c,
            0x76, 0x05,
        ]);
        assert_eq!(aes_kf1(key0, 1), expected);
    }
}
