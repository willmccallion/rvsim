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

use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx};

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

/// AES S-box (forward).
const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// AES inverse S-box.
const AES_INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

/// AES round constants for key schedule (Rcon[i] for i=1..10).
const AES_RCON: [u8; 11] = [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

/// xtime: multiply by x in GF(2^8) with the AES reduction polynomial 0x11b.
#[inline]
const fn xtime(b: u8) -> u8 {
    let high = b >> 7;
    (b << 1) ^ (high.wrapping_neg() & 0x1b)
}

/// AES `SubBytes` on a 16-byte state.
fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = AES_SBOX[*b as usize];
    }
}

/// AES `InvSubBytes` on a 16-byte state.
fn inv_sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = AES_INV_SBOX[*b as usize];
    }
}

/// AES `ShiftRows`: rotate row r left by r bytes (column-major state).
fn shift_rows(state: &mut [u8; 16]) {
    let s = *state;
    for c in 0..4 {
        for r in 0..4 {
            state[c * 4 + r] = s[((c + r) % 4) * 4 + r];
        }
    }
}

/// AES `InvShiftRows`: rotate row r right by r bytes.
fn inv_shift_rows(state: &mut [u8; 16]) {
    let s = *state;
    for c in 0..4 {
        for r in 0..4 {
            state[c * 4 + r] = s[((c + 4 - r) % 4) * 4 + r];
        }
    }
}

/// AES `MixColumns` on a single column [s0, s1, s2, s3].
#[inline]
const fn mix_column(c: [u8; 4]) -> [u8; 4] {
    let s0 = c[0];
    let s1 = c[1];
    let s2 = c[2];
    let s3 = c[3];
    [
        xtime(s0) ^ xtime(s1) ^ s1 ^ s2 ^ s3,
        s0 ^ xtime(s1) ^ xtime(s2) ^ s2 ^ s3,
        s0 ^ s1 ^ xtime(s2) ^ xtime(s3) ^ s3,
        xtime(s0) ^ s0 ^ s1 ^ s2 ^ xtime(s3),
    ]
}

/// AES `MixColumns` on full state.
fn mix_columns(state: &mut [u8; 16]) {
    for c in 0..4 {
        let col = [state[c * 4], state[c * 4 + 1], state[c * 4 + 2], state[c * 4 + 3]];
        let r = mix_column(col);
        state[c * 4..c * 4 + 4].copy_from_slice(&r);
    }
}

/// AES `InvMixColumns` on a single column.
#[inline]
const fn inv_mix_column(c: [u8; 4]) -> [u8; 4] {
    // M^-1 mult by 0x0e, 0x0b, 0x0d, 0x09 in GF(2^8).
    let s0 = c[0];
    let s1 = c[1];
    let s2 = c[2];
    let s3 = c[3];
    [
        gf_mul(0x0e, s0) ^ gf_mul(0x0b, s1) ^ gf_mul(0x0d, s2) ^ gf_mul(0x09, s3),
        gf_mul(0x09, s0) ^ gf_mul(0x0e, s1) ^ gf_mul(0x0b, s2) ^ gf_mul(0x0d, s3),
        gf_mul(0x0d, s0) ^ gf_mul(0x09, s1) ^ gf_mul(0x0e, s2) ^ gf_mul(0x0b, s3),
        gf_mul(0x0b, s0) ^ gf_mul(0x0d, s1) ^ gf_mul(0x09, s2) ^ gf_mul(0x0e, s3),
    ]
}

/// AES `InvMixColumns` on full state.
fn inv_mix_columns(state: &mut [u8; 16]) {
    for c in 0..4 {
        let col = [state[c * 4], state[c * 4 + 1], state[c * 4 + 2], state[c * 4 + 3]];
        let r = inv_mix_column(col);
        state[c * 4..c * 4 + 4].copy_from_slice(&r);
    }
}

/// GF(2^8) multiply with AES reduction polynomial.
#[inline]
const fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut acc: u8 = 0;
    let mut i = 0;
    while i < 8 {
        if b & 1 != 0 {
            acc ^= a;
        }
        let high = a >> 7;
        a = (a << 1) ^ (high.wrapping_neg() & 0x1b);
        b >>= 1;
        i += 1;
    }
    acc
}

/// XOR `a` with `b` in-place.
#[inline]
fn xor_state(a: &mut [u8; 16], b: &[u8; 16]) {
    for i in 0..16 {
        a[i] ^= b[i];
    }
}

/// Convert 4 little-endian u32 words to a 16-byte state (state[col*4 + row]).
fn words_to_state(w: [u32; 4]) -> [u8; 16] {
    let mut s = [0u8; 16];
    for (i, word) in w.iter().enumerate() {
        s[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    s
}

/// Convert 16-byte state back to 4 little-endian u32 words.
fn state_to_words(s: [u8; 16]) -> [u32; 4] {
    let mut w = [0u32; 4];
    for i in 0..4 {
        w[i] = u32::from_le_bytes([s[i * 4], s[i * 4 + 1], s[i * 4 + 2], s[i * 4 + 3]]);
    }
    w
}

/// AES middle-round encryption: state = AddRoundKey(MixColumns(ShiftRows(SubBytes(state))), key)
fn aes_round_enc(state: [u32; 4], key: [u32; 4]) -> [u32; 4] {
    let mut s = words_to_state(state);
    sub_bytes(&mut s);
    shift_rows(&mut s);
    mix_columns(&mut s);
    let k = words_to_state(key);
    xor_state(&mut s, &k);
    state_to_words(s)
}

/// AES final-round encryption (no `MixColumns`).
fn aes_round_enc_final(state: [u32; 4], key: [u32; 4]) -> [u32; 4] {
    let mut s = words_to_state(state);
    sub_bytes(&mut s);
    shift_rows(&mut s);
    let k = words_to_state(key);
    xor_state(&mut s, &k);
    state_to_words(s)
}

/// AES middle-round decryption (per Zvkned spec — equivalent inverse round):
/// state = InvMixColumns(AddRoundKey(InvSubBytes(InvShiftRows(state)), key))
fn aes_round_dec(state: [u32; 4], key: [u32; 4]) -> [u32; 4] {
    let mut s = words_to_state(state);
    inv_shift_rows(&mut s);
    inv_sub_bytes(&mut s);
    let k = words_to_state(key);
    xor_state(&mut s, &k);
    inv_mix_columns(&mut s);
    state_to_words(s)
}

/// AES final-round decryption (no `InvMixColumns`).
fn aes_round_dec_final(state: [u32; 4], key: [u32; 4]) -> [u32; 4] {
    let mut s = words_to_state(state);
    inv_shift_rows(&mut s);
    inv_sub_bytes(&mut s);
    let k = words_to_state(key);
    xor_state(&mut s, &k);
    state_to_words(s)
}

/// AES round-zero: just XOR with key.
const fn aes_round_zero(state: [u32; 4], key: [u32; 4]) -> [u32; 4] {
    [state[0] ^ key[0], state[1] ^ key[1], state[2] ^ key[2], state[3] ^ key[3]]
}

/// `SubWord`: AES S-box applied to each byte of a u32.
#[inline]
const fn sub_word(w: u32) -> u32 {
    let bytes = w.to_le_bytes();
    u32::from_le_bytes([
        AES_SBOX[bytes[0] as usize],
        AES_SBOX[bytes[1] as usize],
        AES_SBOX[bytes[2] as usize],
        AES_SBOX[bytes[3] as usize],
    ])
}

/// `RotWord`: rotate a u32 right by 8 bits (= rotate the 4 bytes left by 1).
#[inline]
const fn rot_word(w: u32) -> u32 {
    w.rotate_right(8)
}

/// AES-128 forward key schedule round (Zvkned §vaeskf1).
///
/// Per the RVV crypto spec, the round number comes from `uimm[3:0]`
/// (uimm[4] is reserved/ignored). The valid range is 1..=10; if the
/// 4-bit value falls outside that range, the spec says the round
/// number is `uimm[3:0] XOR 0x8`.
fn aes_kf1(prev_key: [u32; 4], rnd: u32) -> [u32; 4] {
    let mut r = rnd & 0xf;
    if !(1..=10).contains(&r) {
        r ^= 0x8;
    }
    let r = r as usize;
    let temp = sub_word(rot_word(prev_key[3])) ^ u32::from(AES_RCON[r]);
    let w0 = prev_key[0] ^ temp;
    let w1 = prev_key[1] ^ w0;
    let w2 = prev_key[2] ^ w1;
    let w3 = prev_key[3] ^ w2;
    [w0, w1, w2, w3]
}

/// AES-256 forward key schedule round (Zvkned §vaeskf2).
///
/// Takes the current round key (vd, 4 words) and the previous round key
/// (vs2, 4 words) and a round number `rnd` (2–14). The round number comes
/// from `uimm[3:0]` (uimm[4] is reserved). Out-of-range values are
/// normalised by XOR with 0x8.
///
/// Even rounds apply SubWord(RotWord) ⊕ Rcon, odd rounds just `SubWord`
/// (no rotate, no Rcon).
fn aes_kf2(curr_key: [u32; 4], prev_key: [u32; 4], rnd: u32) -> [u32; 4] {
    let mut r = rnd & 0xf;
    if !(2..=14).contains(&r) {
        r ^= 0x8;
    }
    let temp = if r.is_multiple_of(2) {
        let rcon_idx = ((r / 2) as usize).min(AES_RCON.len() - 1);
        sub_word(rot_word(prev_key[3])) ^ u32::from(AES_RCON[rcon_idx])
    } else {
        sub_word(prev_key[3])
    };
    let w0 = curr_key[0] ^ temp;
    let w1 = curr_key[1] ^ w0;
    let w2 = curr_key[2] ^ w1;
    let w3 = curr_key[3] ^ w2;
    [w0, w1, w2, w3]
}

/// Reverse bits within each byte of a u32. Matches Spike's `ZVK_BREV8_32` macro.
#[inline]
const fn brev8_u32(mut x: u32) -> u32 {
    x = ((x & 0x5555_5555) << 1) | ((x & 0xaaaa_aaaa) >> 1);
    x = ((x & 0x3333_3333) << 2) | ((x & 0xcccc_cccc) >> 2);
    x = ((x & 0x0f0f_0f0f) << 4) | ((x & 0xf0f0_f0f0) >> 4);
    x
}

/// Apply BREV8 to each lane of a [u32; 4] element group.
#[inline]
const fn brev8_u32x4(x: [u32; 4]) -> [u32; 4] {
    [brev8_u32(x[0]), brev8_u32(x[1]), brev8_u32(x[2]), brev8_u32(x[3])]
}

/// `multiplier * multiplicand` in GF(2^128) using the GHASH (NIST GCM)
/// convention: BREV8 the operands, run a left-shift carry-less multiply
/// with reduction polynomial 0x87 (x^128 + x^7 + x^2 + x + 1), then BREV8
/// the result. Mirrors Spike's vgmul/vghsh inner loop verbatim.
fn gf128_mul(multiplier: [u32; 4], multiplicand: [u32; 4]) -> [u32; 4] {
    let y = brev8_u32x4(multiplier);
    let mut h = brev8_u32x4(multiplicand);
    let mut z = [0u32; 4];

    for bit in 0..128 {
        let word = bit / 32;
        let bit_in_word = bit % 32;
        if (y[word] >> bit_in_word) & 1 == 1 {
            for i in 0..4 {
                z[i] ^= h[i];
            }
        }

        let reduce = (h[3] >> 31) & 1 == 1;
        // 128-bit left shift treating h as h[3]:h[2]:h[1]:h[0] (high → low).
        let h_full = (u128::from(h[3]) << 96)
            | (u128::from(h[2]) << 64)
            | (u128::from(h[1]) << 32)
            | u128::from(h[0]);
        let shifted = h_full << 1;
        h[0] = shifted as u32;
        h[1] = (shifted >> 32) as u32;
        h[2] = (shifted >> 64) as u32;
        h[3] = (shifted >> 96) as u32;
        if reduce {
            h[0] ^= 0x87;
        }
    }

    brev8_u32x4(z)
}

/// SHA-256 constants K[0..64] (round constants). Software pre-adds these
/// to the message words and supplies the sum via vs1; the hardware path
/// does not use this array directly. Kept here for documentation.
#[allow(dead_code)]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[inline]
const fn sha256_sigma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}
#[inline]
const fn sha256_sigma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}
#[inline]
const fn sha256_sum0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}
#[inline]
const fn sha256_sum1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}
#[inline]
const fn sha256_ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}
#[inline]
const fn sha256_maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

/// vsha2ms.vv — SHA-256 message scheduling. Per Zvknha:
///   vd  = {W3,  W2,  W1,  W0 } (vd[0]=W0, vd[3]=W3)
///   vs2 = {W11, W10, W9,  W4 } (vs2[0]=W4, vs2[3]=W11)
///   vs1 = {W15, W14, W13, W12} (vs1[0]=W12, vs1[3]=W15)
/// Output replaces vd with {W19, W18, W17, W16}.
const fn sha256_ms(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    let w0 = vd[0];
    let w1 = vd[1];
    let w2 = vd[2];
    let w3 = vd[3];
    let w4 = vs2[0];
    let w9 = vs2[1];
    let w10 = vs2[2];
    let w11 = vs2[3];
    let w12 = vs1[0];
    let w14 = vs1[2];
    let w15 = vs1[3];

    let w16 = sha256_sigma1(w14).wrapping_add(w9).wrapping_add(sha256_sigma0(w1)).wrapping_add(w0);
    let w17 = sha256_sigma1(w15).wrapping_add(w10).wrapping_add(sha256_sigma0(w2)).wrapping_add(w1);
    let w18 = sha256_sigma1(w16).wrapping_add(w11).wrapping_add(sha256_sigma0(w3)).wrapping_add(w2);
    let w19 = sha256_sigma1(w17).wrapping_add(w12).wrapping_add(sha256_sigma0(w4)).wrapping_add(w3);
    [w16, w17, w18, w19]
}

/// One SHA-256 round step on the (a..h) state, mutating in place per FIPS-180-4.
#[inline]
const fn sha256_round(state: &mut [u32; 8], kw: u32) {
    let [a, b, c, d, e, f, g, h] = *state;
    let t1 = h.wrapping_add(sha256_sum1(e)).wrapping_add(sha256_ch(e, f, g)).wrapping_add(kw);
    let t2 = sha256_sum0(a).wrapping_add(sha256_maj(a, b, c));
    *state = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
}

/// vsha2cl.vv / vsha2ch.vv share state layout per Zvknha:
///   vd  = {c, d, g, h}     (vd[0]=h, vd[1]=g, vd[2]=d, vd[3]=c)
///   vs2 = {a, b, e, f}     (vs2[0]=f, vs2[1]=e, vs2[2]=b, vs2[3]=a)
///   vs1 = {kw3, kw2, kw1, kw0}
/// .vsha2cl runs 2 compression rounds with kw0 then kw1; .vsha2ch uses
/// kw2 then kw3. After two rounds the destination is rewritten to the
/// updated low half {a', b', e', f'}.
fn sha256_compress(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4], kw_indices: [usize; 2]) -> [u32; 4] {
    let h = vd[0];
    let g = vd[1];
    let d = vd[2];
    let c = vd[3];
    let f = vs2[0];
    let e = vs2[1];
    let b = vs2[2];
    let a = vs2[3];

    let mut state = [a, b, c, d, e, f, g, h];
    for &idx in &kw_indices {
        sha256_round(&mut state, vs1[idx]);
    }
    [state[5], state[4], state[1], state[0]]
}

#[inline]
fn sha256_compress_low(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    sha256_compress(vd, vs2, vs1, [0, 1])
}

#[inline]
fn sha256_compress_high(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    sha256_compress(vd, vs2, vs1, [2, 3])
}

/// SM3 P0 / P1 / FF / GG functions per GB/T 32905-2016.
#[inline]
const fn sm3_p0(x: u32) -> u32 {
    x ^ x.rotate_left(9) ^ x.rotate_left(17)
}
#[inline]
const fn sm3_p1(x: u32) -> u32 {
    x ^ x.rotate_left(15) ^ x.rotate_left(23)
}
#[inline]
const fn sm3_ff(x: u32, y: u32, z: u32, j: u32) -> u32 {
    if j < 16 { x ^ y ^ z } else { (x & y) | (x & z) | (y & z) }
}
#[inline]
const fn sm3_gg(x: u32, y: u32, z: u32, j: u32) -> u32 {
    if j < 16 { x ^ y ^ z } else { (x & y) | (!x & z) }
}
#[inline]
const fn sm3_t(j: u32) -> u32 {
    if j < 16 { 0x79CC4519 } else { 0x7A879D8A }
}

/// SM3 stores its state and message words byte-swapped in the vector
/// register relative to the algorithm's natural u32 representation. Both
/// extraction and storage apply `swap_bytes` per word to match spike's
/// `EXTRACT/SET_EGU32x8_WORDS_BE_BSWAP` macros.
#[inline]
const fn bswap32(x: u32) -> u32 {
    x.swap_bytes()
}

/// vsm3me.vv — SM3 message expansion. Per Zvksh:
///   vs1 = {W7..W0}  (vs1[i] holds W[i] BSWAP'd)
///   vs2 = {W15..W8} (vs2[i] holds W[8+i] BSWAP'd)
/// Output replaces vd with {W23..W16}.
fn sm3_me(vs1: [u32; 8], vs2: [u32; 8]) -> [u32; 8] {
    let mut w = [0u32; 24];
    for i in 0..8 {
        w[i] = bswap32(vs1[i]);
        w[i + 8] = bswap32(vs2[i]);
    }
    for j in 16..24 {
        w[j] = sm3_p1(w[j - 16] ^ w[j - 9] ^ w[j - 3].rotate_left(15))
            ^ w[j - 13].rotate_left(7)
            ^ w[j - 6];
    }
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = bswap32(w[16 + i]);
    }
    out
}

/// vsm3c.vi — SM3 compression. Per Zvksh:
///   vd  = {H,G,F,E,D,C,B,A}  (state, BSWAP'd; vd[0]=`A_bswap`, vd[7]=`H_bswap`)
///   vs2 = {_,_,w5,w4,_,_,w1,w0} (only positions 0,1,4,5 used)
/// Two rounds j = 2*rnd and j+1; output rewrites vd as
/// {G1, G2, E1, E2, C1, C2, A1, A2} (BSWAP'd).
const fn sm3_c(vd: [u32; 8], vs2: [u32; 8], rnd: u32) -> [u32; 8] {
    let a = bswap32(vd[0]);
    let b = bswap32(vd[1]);
    let c = bswap32(vd[2]);
    let d = bswap32(vd[3]);
    let e = bswap32(vd[4]);
    let f = bswap32(vd[5]);
    let g = bswap32(vd[6]);
    let h = bswap32(vd[7]);

    let w0 = bswap32(vs2[0]);
    let w1 = bswap32(vs2[1]);
    let w4 = bswap32(vs2[4]);
    let w5 = bswap32(vs2[5]);
    let x0 = w0 ^ w4;
    let x1 = w1 ^ w5;

    // Round j = 2*rnd
    let j = 2 * rnd;
    let ss1 =
        a.rotate_left(12).wrapping_add(e).wrapping_add(sm3_t(j).rotate_left(j % 32)).rotate_left(7);
    let ss2 = ss1 ^ a.rotate_left(12);
    let tt1 = sm3_ff(a, b, c, j).wrapping_add(d).wrapping_add(ss2).wrapping_add(x0);
    let tt2 = sm3_gg(e, f, g, j).wrapping_add(h).wrapping_add(ss1).wrapping_add(w0);
    let d1 = c;
    let c1 = b.rotate_left(9);
    let b1 = a;
    let a1 = tt1;
    let h1 = g;
    let g1 = f.rotate_left(19);
    let f1 = e;
    let e1 = sm3_p0(tt2);

    // Round j+1
    let j = j + 1;
    let ss1 = a1
        .rotate_left(12)
        .wrapping_add(e1)
        .wrapping_add(sm3_t(j).rotate_left(j % 32))
        .rotate_left(7);
    let ss2 = ss1 ^ a1.rotate_left(12);
    let tt1 = sm3_ff(a1, b1, c1, j).wrapping_add(d1).wrapping_add(ss2).wrapping_add(x1);
    let tt2 = sm3_gg(e1, f1, g1, j).wrapping_add(h1).wrapping_add(ss1).wrapping_add(w1);
    let c2 = b1.rotate_left(9);
    let a2 = tt1;
    let g2 = f1.rotate_left(19);
    let e2 = sm3_p0(tt2);

    [
        bswap32(a2),
        bswap32(a1),
        bswap32(c2),
        bswap32(c1),
        bswap32(e2),
        bswap32(e1),
        bswap32(g2),
        bswap32(g1),
    ]
}

/// SM4 S-box.
const SM4_SBOX: [u8; 256] = [
    0xd6, 0x90, 0xe9, 0xfe, 0xcc, 0xe1, 0x3d, 0xb7, 0x16, 0xb6, 0x14, 0xc2, 0x28, 0xfb, 0x2c, 0x05,
    0x2b, 0x67, 0x9a, 0x76, 0x2a, 0xbe, 0x04, 0xc3, 0xaa, 0x44, 0x13, 0x26, 0x49, 0x86, 0x06, 0x99,
    0x9c, 0x42, 0x50, 0xf4, 0x91, 0xef, 0x98, 0x7a, 0x33, 0x54, 0x0b, 0x43, 0xed, 0xcf, 0xac, 0x62,
    0xe4, 0xb3, 0x1c, 0xa9, 0xc9, 0x08, 0xe8, 0x95, 0x80, 0xdf, 0x94, 0xfa, 0x75, 0x8f, 0x3f, 0xa6,
    0x47, 0x07, 0xa7, 0xfc, 0xf3, 0x73, 0x17, 0xba, 0x83, 0x59, 0x3c, 0x19, 0xe6, 0x85, 0x4f, 0xa8,
    0x68, 0x6b, 0x81, 0xb2, 0x71, 0x64, 0xda, 0x8b, 0xf8, 0xeb, 0x0f, 0x4b, 0x70, 0x56, 0x9d, 0x35,
    0x1e, 0x24, 0x0e, 0x5e, 0x63, 0x58, 0xd1, 0xa2, 0x25, 0x22, 0x7c, 0x3b, 0x01, 0x21, 0x78, 0x87,
    0xd4, 0x00, 0x46, 0x57, 0x9f, 0xd3, 0x27, 0x52, 0x4c, 0x36, 0x02, 0xe7, 0xa0, 0xc4, 0xc8, 0x9e,
    0xea, 0xbf, 0x8a, 0xd2, 0x40, 0xc7, 0x38, 0xb5, 0xa3, 0xf7, 0xf2, 0xce, 0xf9, 0x61, 0x15, 0xa1,
    0xe0, 0xae, 0x5d, 0xa4, 0x9b, 0x34, 0x1a, 0x55, 0xad, 0x93, 0x32, 0x30, 0xf5, 0x8c, 0xb1, 0xe3,
    0x1d, 0xf6, 0xe2, 0x2e, 0x82, 0x66, 0xca, 0x60, 0xc0, 0x29, 0x23, 0xab, 0x0d, 0x53, 0x4e, 0x6f,
    0xd5, 0xdb, 0x37, 0x45, 0xde, 0xfd, 0x8e, 0x2f, 0x03, 0xff, 0x6a, 0x72, 0x6d, 0x6c, 0x5b, 0x51,
    0x8d, 0x1b, 0xaf, 0x92, 0xbb, 0xdd, 0xbc, 0x7f, 0x11, 0xd9, 0x5c, 0x41, 0x1f, 0x10, 0x5a, 0xd8,
    0x0a, 0xc1, 0x31, 0x88, 0xa5, 0xcd, 0x7b, 0xbd, 0x2d, 0x74, 0xd0, 0x12, 0xb8, 0xe5, 0xb4, 0xb0,
    0x89, 0x69, 0x97, 0x4a, 0x0c, 0x96, 0x77, 0x7e, 0x65, 0xb9, 0xf1, 0x09, 0xc5, 0x6e, 0xc6, 0x84,
    0x18, 0xf0, 0x7d, 0xec, 0x3a, 0xdc, 0x4d, 0x20, 0x79, 0xee, 0x5f, 0x3e, 0xd7, 0xcb, 0x39, 0x48,
];

/// SM4 round constants CK[0..32] (little-endian per byte).
const SM4_CK: [u32; 32] = [
    0x00070E15, 0x1C232A31, 0x383F464D, 0x545B6269, 0x70777E85, 0x8C939AA1, 0xA8AFB6BD, 0xC4CBD2D9,
    0xE0E7EEF5, 0xFC030A11, 0x181F262D, 0x343B4249, 0x50575E65, 0x6C737A81, 0x888F969D, 0xA4ABB2B9,
    0xC0C7CED5, 0xDCE3EAF1, 0xF8FF060D, 0x141B2229, 0x30373E45, 0x4C535A61, 0x686F767D, 0x848B9299,
    0xA0A7AEB5, 0xBCC3CAD1, 0xD8DFE6ED, 0xF4FB0209, 0x10171E25, 0x2C333A41, 0x484F565D, 0x646B7279,
];

/// SM4 system parameter FK[0..4].
const SM4_FK: [u32; 4] = [0xA3B1BAC6, 0x56AA3350, 0x677D9197, 0xB27022DC];

#[inline]
const fn sm4_tau(a: u32) -> u32 {
    let bytes = a.to_le_bytes();
    u32::from_le_bytes([
        SM4_SBOX[bytes[0] as usize],
        SM4_SBOX[bytes[1] as usize],
        SM4_SBOX[bytes[2] as usize],
        SM4_SBOX[bytes[3] as usize],
    ])
}

/// SM4 linear transform L for round function.
#[inline]
const fn sm4_l(b: u32) -> u32 {
    b ^ b.rotate_left(2) ^ b.rotate_left(10) ^ b.rotate_left(18) ^ b.rotate_left(24)
}

/// SM4 linear transform L' for key schedule.
#[inline]
const fn sm4_l_prime(b: u32) -> u32 {
    b ^ b.rotate_left(13) ^ b.rotate_left(23)
}

/// vsm4r — SM4 round function applied to 4 elements.
///
/// vd = {X3, X2, X1, X0}, vs2 = {RK3, RK2, RK1, RK0}.
/// Output: {X7, X6, X5, X4} where Xi+4 = Xi ^ T(Xi+1 ^ Xi+2 ^ Xi+3 ^ `RKi`).
fn sm4_r(vd: [u32; 4], vs2: [u32; 4]) -> [u32; 4] {
    let mut x = [vd[0], vd[1], vd[2], vd[3], 0, 0, 0, 0];
    for i in 0..4 {
        let t = x[i + 1] ^ x[i + 2] ^ x[i + 3] ^ vs2[i];
        x[i + 4] = x[i] ^ sm4_l(sm4_tau(t));
    }
    [x[4], x[5], x[6], x[7]]
}

/// vsm4k.vi — SM4 key expansion.
///
/// vd = current 4 round keys {RK3, RK2, RK1, RK0}, rnd selects which 4-round
/// chunk to produce. Output: next 4 round keys.
/// On the first call (rnd=0), vd should be MK ^ FK (caller responsibility?).
/// Actually per spec: vd contains the previous 4 K-values; output is next 4.
///
/// Per Zvksed: rnd comes from a 5-bit immediate but only bits [2:0] are used
/// (uimm[4:3] are reserved). Mask here so an out-of-spec encoding can't index
/// past `SM4_CK[32]`.
fn sm4_k(vd: [u32; 4], rnd: u32) -> [u32; 4] {
    let mut k = [vd[0], vd[1], vd[2], vd[3], 0, 0, 0, 0];
    let base = ((rnd & 0x7) * 4) as usize;
    for i in 0..4 {
        let t = k[i + 1] ^ k[i + 2] ^ k[i + 3] ^ SM4_CK[base + i];
        k[i + 4] = k[i] ^ sm4_l_prime(sm4_tau(t));
    }
    [k[4], k[5], k[6], k[7]]
}

// Suppress unused-constant warning for FK (kept for completeness; software
// callers may want it for the very-first round key derivation MK ^ FK).
#[allow(dead_code)]
const _SM4_FK_USED: [u32; 4] = SM4_FK;

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
